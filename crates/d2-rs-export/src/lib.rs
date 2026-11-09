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

use d2_rs_core::{Graph, NodeId, Point, Rect, ShapeKind, theme};
use d2_rs_language::compiler::SourceNodeStyle;
use d2_rs_render::RenderOptions;
use d2_rs_tala::LayoutResult;
use image_webp::WebPDecoder;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufReader, Cursor};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub mod ascii_target;
mod bundled_emoji;
mod colr_plan;
mod document_links;
mod font_fallback;
mod font_shaping;
mod gif_encode;
mod gif_pixels;
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
pub use document_links::page_fill as document_page_fill;
pub use paged_admission::{GifBoard, GifFontResources};
pub use pptx::PptxDocumentProperties;
mod scanline;
#[cfg(test)]
mod sketch_contract;
use d2_rs_math::image_svg::svg_asset_codec;
use d2_rs_math::image_svg::svg_asset_xml;
mod svg_import;
#[cfg(all(test, feature = "oracle-fixtures"))]
use d2_rs_math::image_svg::svg_import_envelope;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod svg_import_geometry;
mod svg_precision;

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
        ExportFormat::Svg => d2_rs_render::render_svg_with_sketch_checked(
            graph, result, labels, styles, options, sketch,
        )
        .map_err(|error| ExportError::Render(error.to_string())),
        ExportFormat::Ascii => {
            render_ascii_with_options(graph, result, labels, styles, &options, sketch)
        }
        ExportFormat::Png => {
            // encodePNGBands admits the scene before its first row. Rendering
            // first and admitting afterwards returns the same result (the
            // admission error wins over any render error) while the scene
            // reuses the renderer's freed memory instead of raising its peak.
            let admission_options = options.clone();
            let rendered =
                render_png_with_options(graph, result, labels, styles, options, sketch, 2);
            paged_admission::png_preflight(
                graph,
                result,
                labels,
                styles,
                &admission_options,
                sketch,
            )?;
            rendered
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

/// Rasterizes the semantic SVG target with the native resvg/tiny-skia path.
///
/// This keeps PNG generation in-process and independent of a browser or Go
/// helper.  The SVG remains the semantic source of truth, so shape, marker,
/// clipping, and text changes are shared by both targets.
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
    let mut stream: Option<go_png::RgbaRowStream> = None;
    let mut encode_band = |width: usize, height: usize, band: &mut [u8]| {
        unpremultiply_rgba(band);
        stream
            .get_or_insert_with(|| go_png::RgbaRowStream::new(width as u32, height as u32))
            .push_rows(band);
    };
    let (width, height, rgba) = render_paged_frame(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        pixel_scale,
        budget,
        Some(&mut encode_band),
    )?;
    if rgba.is_empty()
        && let Some(stream) = stream.take()
    {
        return Ok(stream.finish());
    }
    drop(stream);
    go_png::encode(width, height, &rgba, 4).map_err(ExportError::Png)
}

/// A paged signature shared by `render_paged_png_rgba_at_scale` and
/// `render_paged_pdf_page_at_scale`.
pub type PagedFrameRenderer<T> = fn(
    &Graph,
    &LayoutResult,
    &BTreeMap<NodeId, String>,
    &BTreeMap<NodeId, SourceNodeStyle>,
    RenderOptions,
    bool,
    u32,
    &mut PagedFrameBudget,
) -> Result<T, ExportError>;

/// One rendered PDF page image, ready for
/// `package_pdf_page_rasters_with_navigation_metadata_for_theme`.
#[derive(Clone, Debug)]
pub struct PdfPage(PdfRasterPage);

/// `render_paged_png_rgba_at_scale` for a PDF page. FPDF parses the page PNG
/// back into color and alpha planes; PNG is lossless, so those planes are the
/// frame's own unpremultiplied bytes. Only an opaque page, whose RGB PNG
/// image data FPDF embeds as is, still needs the PNG.
pub fn render_paged_pdf_page_at_scale(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
    budget: &mut PagedFrameBudget,
) -> Result<PdfPage, ExportError> {
    let (width, height, rgba) = render_paged_frame(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        pixel_scale,
        budget,
        None,
    )?;
    pdf_page_from_rgba(width, height, rgba)?
        .compacted()
        .map(PdfPage)
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

/// The frame of a paged board: unpremultiplied RGBA over a transparent page.
/// `band_sink` encodes it band by band, bounding its effect layers; a PDF page
/// keeps the whole-frame pass, since its page planes outweigh those layers.
#[allow(clippy::too_many_arguments)]
fn render_paged_frame(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
    budget: &mut PagedFrameBudget,
    band_sink: Option<scanline::BandSink<'_>>,
) -> Result<(u32, u32, Vec<u8>), ExportError> {
    if budget.total_boards == 0 || budget.remaining_pixels == 0 {
        return Err(ExportError::Render(
            "paged export exhausted its operation budget".into(),
        ));
    }
    if pixel_scale == 0 {
        return Err(ExportError::Png("pixel scale must be positive".into()));
    }
    // Go renders every paged board from its typed scene. The incremental Rust
    // port still uses the SVG bridge for ordinary boards, but labels with
    // default-ignorable controls require the scene shaper: XML/usvg can
    // otherwise republish a different mixed-script run at the 1x paged scale.
    if paged_scene_shaping_required(graph, labels) {
        let (width, height, rgba) = paged_admission::render_frame(
            graph,
            result,
            labels,
            styles,
            &options,
            sketch,
            pixel_scale,
            budget,
        )?;
        budget.remaining_pixels -= u64::from(width) * u64::from(height);
        return Ok((width, height, rgba));
    }
    paged_admission::preflight(
        graph,
        result,
        labels,
        styles,
        &options,
        sketch,
        pixel_scale,
        budget,
    )?;
    let (width, height, mut rgba) = rasterize_frame(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        pixel_scale,
        None,
        true,
        false,
        Some(budget.remaining_pixels.min(64 * 1024 * 1024)),
        band_sink,
    )?;
    if !rgba.is_empty() {
        // An unstreamed frame, or the resvg fallback's.
        unpremultiply_rgba(&mut rgba);
    }
    budget.remaining_pixels -= u64::from(width) * u64::from(height);
    Ok((width, height, rgba))
}

fn paged_scene_shaping_required(graph: &Graph, labels: &BTreeMap<NodeId, String>) -> bool {
    let needs_scene = |text: &str| text.chars().any(font_shaping::ignorable);
    labels.values().any(|text| needs_scene(text))
        || graph
            .edge_labels
            .iter()
            .flatten()
            .any(|label| needs_scene(&label.text))
        || graph.edge_arrowhead_labels.iter().any(|labels| {
            labels
                .source
                .as_ref()
                .is_some_and(|label| needs_scene(&label.text))
                || labels
                    .target
                    .as_ref()
                    .is_some_and(|label| needs_scene(&label.text))
        })
        || graph.legend.as_ref().is_some_and(|legend| {
            needs_scene(&legend.label)
                || legend.shapes.iter().any(|shape| needs_scene(&shape.label))
                || legend.connections.iter().any(|connection| {
                    needs_scene(&connection.label)
                        || needs_scene(&connection.source_label)
                        || needs_scene(&connection.target_label)
                })
                || legend.source_graph.as_ref().is_some_and(|source| {
                    needs_scene(&source.label)
                        || source.labels.values().any(|text| needs_scene(text))
                })
        })
}

fn unpremultiply_rgba(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 {
            pixel[..3].fill(0);
        } else if alpha < 255 {
            for channel in &mut pixel[..3] {
                // Match d2raster's rgbaToNRGBAInPlaceBounds conversion. Go
                // scales by 0xffff, divides by alpha, then shifts by eight;
                // rounding to nearest here changes semitransparent edge RGB
                // values by one and shows up after PDF soft-mask compositing.
                *channel = ((u32::from(*channel) * 0xffff / alpha) >> 8).min(255) as u8;
            }
        }
    }
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
    render_png_with_animation_time(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        pixel_scale,
        None,
        false,
    )
}

fn render_png_with_animation_time(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
    animation_time_ns: Option<i64>,
    gif_frame: bool,
) -> Result<Vec<u8>, ExportError> {
    // D2's PNG encoder renders the logical SVG at a fixed 2x pixel density.
    // Keep the SVG dimensions/options unchanged while matching the release
    // binary's decoded pixel dimensions. d2cli writes PNG through
    // d2raster.RenderBands (raster_png_output.go), unlike GIF and paged
    // frames, so only this path emulates its band destinations.
    let _bands = scanline::GoPngBandsScope::new();
    // Like d2cli's rasterPNGBandEncoder, encode the native frame as its bands
    // are finished rather than retaining the whole frame. Composited over
    // white, every row is opaque, which selects truecolor output.
    // A second thread filters and compresses each finished band while the
    // next one renders; the rendezvous queue keeps one band in flight.
    let (frame, encoded) = std::thread::scope(|scope| {
        let (bands, queue) = std::sync::mpsc::sync_channel::<(usize, usize, Vec<u8>)>(0);
        let (reuse, recycled) = std::sync::mpsc::channel();
        let encoder = scope.spawn(move || {
            let mut stream: Option<go_png::RowStream> = None;
            for (width, height, mut band) in queue {
                composite_over_white(&mut band);
                stream
                    .get_or_insert_with(|| go_png::RowStream::new(width as u32, height as u32, 3))
                    .push_rows(&band, 4);
                let _ = reuse.send(band);
            }
            stream.map(go_png::RowStream::finish)
        });
        // A frame that arrives as one band is encoded in place: overlapping
        // it with nothing would only cost a copy of the frame.
        let mut whole: Option<Vec<u8>> = None;
        let mut band_in_flight = false;
        let mut encode_band = |width: usize, height: usize, band: &mut [u8]| {
            if whole.is_none() && band.len() == width * height * 4 {
                composite_over_white(band);
                let mut stream = go_png::RowStream::new(width as u32, height as u32, 3);
                stream.push_rows(band, 4);
                whole = Some(stream.finish());
                return;
            }
            // Reuse the preceding band's storage after the encoder consumes
            // it. Allocating before the rendezvous can otherwise hold two
            // encoder bands alongside the rasterizer's current band.
            let mut buffer = if band_in_flight {
                recycled.recv().expect("PNG band encoder buffer")
            } else {
                band_in_flight = true;
                Vec::new()
            };
            buffer.clear();
            buffer.extend_from_slice(band);
            // The receiver lives until the queue closes; a send cannot fail.
            let _ = bands.send((width, height, buffer));
        };
        let frame = rasterize_frame(
            graph,
            result,
            labels,
            styles,
            options,
            sketch,
            pixel_scale,
            animation_time_ns,
            false,
            gif_frame,
            None,
            Some(&mut encode_band),
        );
        // Closing the queue ends the encoder's loop.
        drop(bands);
        drop(recycled);
        let encoded = encoder.join().expect("PNG band encoder thread");
        (frame, whole.or(encoded))
    });
    let (width, height, rgba) = frame?;
    if rgba.is_empty()
        && let Some(encoded) = encoded
    {
        return Ok(encoded);
    }
    // D2's PNG target composites the scene over the canvas background. Keep
    // PNG output opaque even though the intermediate SVG raster remains RGBA;
    // GIF and PDF use the same conversion below.
    let mut rgba = rgba;
    composite_over_white(&mut rgba);
    if !rgba.len().is_multiple_of(4) {
        return Err(ExportError::Png(
            "raster dimensions do not match the pixel buffer".into(),
        ));
    }
    // The composited buffer is opaque, so the encoder writes RGB rows from it
    // directly instead of retaining a second, RGB copy of the frame.
    go_png::encode(width, height, &rgba, 4).map_err(ExportError::Png)
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
    render_png_with_animation_time(
        graph, result, labels, styles, options, sketch, 1, None, true,
    )
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
            resvg::tiny_skia::Pixmap::decode_png(png).map_err(|error| {
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
    // GIF frames retain the logical SVG pixel density in v0.9.0.
    let (width, height, rgba) = rasterize_with_animation_time(
        graph, result, labels, styles, options, sketch, 1, None, true,
    )?;
    let width_u16 = u16::try_from(width)
        .map_err(|_| ExportError::Gif("SVG width exceeds GIF's 65535-pixel limit".into()))?;
    let height_u16 = u16::try_from(height)
        .map_err(|_| ExportError::Gif("SVG height exceeds GIF's 65535-pixel limit".into()))?;
    let mut rgba = rgba;
    composite_over_white(&mut rgba);
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
    let link_regions = pdf_link_regions(graph, result, labels, styles, &options, sketch)?;
    let (width, height, rgba) = rasterize_with_animation_time_and_background(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        2,
        None,
        true,
        false,
        Some(64 * 1024 * 1024),
    )
    .map_err(|error| match error {
        error @ ExportError::Render(_) => error,
        ExportError::Png(message) => ExportError::Pdf(message),
        other => ExportError::Pdf(other.to_string()),
    })?;
    let page = pdf_raster_from_premultiplied_rgba(width, height, rgba)?;
    encode_pdf_pages_with_links(&[page], &[link_regions], header_rule_is_light)
}

/// Returns the semantic link rectangles in output-page coordinates.  The
/// raster image remains the visual source of truth; these regions restore the
/// interactive part of D2's document target without embedding a browser or
/// changing layout geometry.
fn pdf_link_metadata(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
) -> Result<Vec<DocumentLinkMetadata>, ExportError> {
    let mut source_regions = document_links::target_regions(graph, result, labels, styles)?;
    let has_markdown = styles.values().any(|style| {
        style.markdown || matches!(style.language.as_deref(), Some("md" | "markdown"))
    });
    if source_regions.is_empty() && !has_markdown {
        return Ok(Vec::new());
    }
    let svg_options = RenderOptions {
        force_appendix: true,
        ..options.clone()
    };
    let svg = d2_rs_render::render_svg_with_sketch_checked(
        graph,
        result,
        labels,
        styles,
        svg_options,
        sketch,
    )
    .map_err(|error| ExportError::Pdf(error.to_string()))?;
    let markdown = d2_rs_render::markdown_scene::collect_svg(
        &svg,
        d2_rs_render::markdown_scene::LinkBudget::default(),
        true,
    )
    .map_err(ExportError::Pdf)?;
    // Scene construction adds titled Markdown links and edge tooltips to its
    // static appendix even when the intermediate SVG has no appendix group.
    let scene_appendix = !markdown.appendix.is_empty()
        || graph.edges().any(|(id, _)| {
            graph
                .edge_tooltip(id)
                .is_some_and(|value| !value.is_empty())
        });
    source_regions.extend(
        markdown
            .links
            .into_iter()
            .filter(|region| !region.url.is_empty())
            .map(|region| DocumentLinkMetadata {
                tooltip: region.tooltip,
                board_target: !region.target.is_empty(),
                region: DocumentLinkRegion {
                    href: region.url,
                    rect: Rect {
                        origin: Point {
                            x: region.r#box.x,
                            y: region.r#box.y,
                        },
                        size: d2_rs_core::Size {
                            width: region.r#box.width,
                            height: region.r#box.height,
                        },
                    },
                },
            }),
    );
    let Some((view_left, view_top, view_width, view_height, outer_width, outer_height)) =
        svg_viewbox_mapping(&svg, scene_appendix)
    else {
        return Err(ExportError::Pdf(
            "could not determine SVG coordinate mapping for PDF links".into(),
        ));
    };
    // The outer SVG dimensions carry the source logical scale. When an
    // appendix resets that scale, its emitted dimensions already reflect it.
    let scale_x = outer_width / view_width;
    let scale_y = outer_height / view_height;
    Ok(source_regions
        .into_iter()
        .map(|mut region| {
            region.rect = Rect {
                origin: Point {
                    x: (region.rect.origin.x - view_left) * scale_x,
                    y: (region.rect.origin.y - view_top) * scale_y,
                },
                size: d2_rs_core::Size {
                    width: region.rect.size.width * scale_x,
                    height: region.rect.size.height * scale_y,
                },
            };
            region
        })
        .collect())
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
    pdf_link_regions(graph, result, labels, styles, &options, sketch)
}

fn pdf_link_regions(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
) -> Result<Vec<DocumentLinkRegion>, ExportError> {
    Ok(
        pdf_link_metadata(graph, result, labels, styles, options, sketch)?
            .into_iter()
            .filter(|m| !m.href.is_empty())
            .map(|m| m.region)
            .collect(),
    )
}

/// Source scene links with URL/board provenance and tooltip text intact.
pub fn document_link_metadata(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
) -> Result<Vec<DocumentLinkMetadata>, ExportError> {
    pdf_link_metadata(graph, result, labels, styles, &options, sketch)
}

fn svg_viewbox_mapping(svg: &[u8], scene_appendix: bool) -> Option<(f64, f64, f64, f64, f64, f64)> {
    let text = std::str::from_utf8(svg).ok()?;
    let document = roxmltree::Document::parse(text).ok()?;
    let root = document.root_element();
    let mut boxes = text
        .match_indices("viewBox=\"")
        .filter_map(|(offset, marker)| {
            let start = offset + marker.len();
            let end = text[start..].find('"')? + start;
            let values = text[start..end]
                .split_whitespace()
                .map(str::parse::<f64>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            (values.len() == 4).then_some(values)
        });
    let outer = boxes.next()?;
    let inner = boxes.next()?;
    // build.go resets logicalScale when the static appendix has rows, even
    // if the intermediate SVG still carries the requested outer size.
    let appendix = scene_appendix
        || document.descendants().any(|node| {
            node.is_element()
                && node
                    .attribute("class")
                    .is_some_and(|classes| classes.split_whitespace().any(|c| c == "appendix"))
        });
    let width = if appendix {
        outer[2]
    } else {
        root.attribute("width")
            .and_then(|v| v.parse().ok())
            .unwrap_or(outer[2])
    };
    let height = if appendix {
        outer[3]
    } else {
        root.attribute("height")
            .and_then(|v| v.parse().ok())
            .unwrap_or(outer[3])
    };
    // paged.go mapPagedLinks uses the same meet viewport as the raster
    // frame. Rounded logical dimensions can give the two axes different
    // ratios; the smaller ratio applies to both, with optional centering.
    let scale = (width / inner[2]).min(height / inner[3]);
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let centered = root
        .attribute("preserveAspectRatio")
        .is_some_and(|value| value.starts_with("xMidYMid"));
    let offset_x = if centered {
        (width - inner[2] * scale) / 2.0
    } else {
        0.0
    };
    let offset_y = if centered {
        (height - inner[3] * scale) / 2.0
    } else {
        0.0
    };
    Some((
        inner[0] - offset_x / scale,
        inner[1] - offset_y / scale,
        inner[2],
        inner[3],
        inner[2] * scale,
        inner[3] * scale,
    ))
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
    let semantic_links = pdf_link_metadata(graph, result, labels, styles, &options, sketch)?;
    let png = render_png_rgba_at_scale(graph, result, labels, styles, options, sketch, 2)?;
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

fn composite_over_white(rgba: &mut [u8]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * alpha + 255 * (255 - alpha)) / 255) as u8;
            }
        }
        pixel[3] = 255;
    }
}

fn pdf_raster_from_premultiplied_rgba(
    width: u32,
    height: u32,
    mut rgba: Vec<u8>,
) -> Result<PdfRasterPage, ExportError> {
    let pixel_count = (width as usize).saturating_mul(height as usize);
    if rgba.len() != pixel_count.saturating_mul(4) {
        return Err(ExportError::Pdf(
            "raster dimensions do not match the pixel buffer".into(),
        ));
    }
    unpremultiply_rgba(&mut rgba);
    let mut rgb = Vec::with_capacity(pixel_count.saturating_mul(3));
    let mut alpha = Vec::with_capacity(pixel_count);
    let mut has_transparency = false;
    for pixel in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
        alpha.push(pixel[3]);
        has_transparency |= pixel[3] != 255;
    }
    Ok(PdfRasterPage {
        width,
        height,
        rgb,
        alpha: has_transparency.then_some(alpha),
        rgb_png_idat: None,
        streams: None,
    })
}

/// Rasterizes the semantic SVG target into an RGBA pixel buffer.
#[cfg(test)]
fn rasterize(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
) -> Result<(u32, u32, Vec<u8>), ExportError> {
    rasterize_with_animation_time(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        pixel_scale,
        None,
        false,
    )
}

fn rasterize_with_animation_time(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
    animation_time_ns: Option<i64>,
    gif_frame: bool,
) -> Result<(u32, u32, Vec<u8>), ExportError> {
    rasterize_with_animation_time_and_background(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        pixel_scale,
        animation_time_ns,
        false,
        gif_frame,
        None,
    )
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

// Source: d2raster/raster.go prepare. Check width, height, then pixels;
// PNG band rendering allows the complete dimension domain; paged callers
// impose a smaller pixel ceiling. GIF has separate schedule admission.
fn validate_raster_frame_dimensions(
    logical_width: f64,
    logical_height: f64,
    pixel_scale: u32,
    max_pixels: u64,
) -> Result<(), ExportError> {
    let width = logical_width * f64::from(pixel_scale);
    let height = logical_height * f64::from(pixel_scale);
    let error = if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        Some("d2raster: invalid scaled dimensions".into())
    } else if width.ceil() > 32_768.0 {
        Some(format!(
            "d2raster: frame width {:.0} exceeds limit 32768",
            width.ceil()
        ))
    } else if height.ceil() > 32_768.0 {
        Some(format!(
            "d2raster: frame height {:.0} exceeds limit 32768",
            height.ceil()
        ))
    } else if width.ceil() as u64 > max_pixels / height.ceil() as u64 {
        Some(format!("d2raster: frame pixels exceed limit {max_pixels}"))
    } else {
        None
    };
    match error {
        Some(message) => Err(ExportError::Render(message)),
        None => Ok(()),
    }
}

fn rasterize_with_animation_time_and_background(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
    animation_time_ns: Option<i64>,
    transparent_background: bool,
    gif_frame: bool,
    paged_max_pixels: Option<u64>,
) -> Result<(u32, u32, Vec<u8>), ExportError> {
    rasterize_frame(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        pixel_scale,
        animation_time_ns,
        transparent_background,
        gif_frame,
        paged_max_pixels,
        None,
    )
}

/// `rasterize_with_animation_time_and_background`, optionally handing the
/// native renderer's frame to `band_sink` band by band. A streamed frame is
/// returned with empty pixels; the resvg fallback always returns its pixels.
fn rasterize_frame(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
    animation_time_ns: Option<i64>,
    transparent_background: bool,
    gif_frame: bool,
    paged_max_pixels: Option<u64>,
    band_sink: Option<scanline::BandSink<'_>>,
) -> Result<(u32, u32, Vec<u8>), ExportError> {
    // Fallback probes read each system font file once per render.
    let _fallback_files = font_fallback::FallbackFileScope::new();
    // D2's scene builder always enables the static link/tooltip appendix for
    // non-interactive raster exports. SVG callers opt into the same path with
    // `--force-appendix`; keeping this here makes PNG/GIF/PDF/PPTX share that
    // source behavior without changing the interactive SVG API.
    validate_positioned_tooltip_scene_fields(graph, styles)?;
    let appendix_mono = options
        .theme_id
        .and_then(theme::palette)
        .is_some_and(|palette| palette.special_rules.mono);
    let imported_assets = validate_imported_svg_assets(graph, styles, &options)?;
    let mut prepared_jpegs = prepare_jpeg_assets(graph, styles, &options, &imported_assets)?;
    let mut asset_handoffs = png_image::HandoffScope::default();
    let mut options = normalize_raster_assets(options, &prepared_jpegs, &mut asset_handoffs);
    // Imported vector resources use the same decoded assets as direct icons.
    // Prepare them before band rendering, as d2raster prepareRasterAsset does.
    for bytes in prepared_jpegs.by_asset.values_mut() {
        if let Some(prepared) = prepare_raster_pixels(bytes, &mut asset_handoffs) {
            *bytes = prepared;
        }
    }
    options.force_appendix = true;
    options.transparent_background = transparent_background;
    let custom_fonts = options.custom_fonts.clone();
    let (svg, raster_sources) = d2_rs_render::render_svg_with_raster_sources_checked(
        graph, result, labels, styles, options, sketch,
    )
    .map_err(|error| ExportError::Render(error.to_string()))?;
    // The SVG target embeds the D2 font assets for browser portability. The
    // native raster path registers those faces directly in resvg's font
    // database, including private aliases for synthetic Markdown axes;
    // discard all CSS @font-face declarations so a data URL cannot create a
    // second, differently-prioritized copy.
    let mut svg_for_native_raster = {
        let svg = std::str::from_utf8(&svg)
            .map_err(|error| ExportError::Png(format!("SVG output was not UTF-8: {error}")))?;
        strip_embedded_font_faces(svg)
    };
    // d2raster prepares its scene before drawing. Retain only the current
    // SVG representation, rather than every intermediate copy of its images
    // and font data throughout the band's draw and encoding.
    drop(svg);
    svg_for_native_raster = configure_native_appendix_font_roles(&svg_for_native_raster);
    // Go's raster path applies the synthetic italic axis through its SVG
    // text backend.  resvg's CSS italic synthesis uses a different slant
    // kernel, even though ordinary regular/bold faces are pixel-identical.
    // Keep the source-shaped `font-style` in exported SVG, but use the
    // v0.9.0-compatible affine form for native raster targets.
    svg_for_native_raster = emulate_go_synthetic_italic(&svg_for_native_raster);
    // Go's native raster renderer samples animated tracks at time zero. The
    // SVG target keeps the connection's base offset in the presentation
    // attributes and animates from zero, so resvg (which does not evaluate
    // SMIL/CSS animation tracks) must see that initial offset explicitly.
    svg_for_native_raster = match animation_time_ns {
        Some(timestamp_ns) => set_animated_dash_offsets(&svg_for_native_raster, Some(timestamp_ns)),
        None => reset_animated_dash_offsets(&svg_for_native_raster),
    };
    #[cfg(feature = "diagnostic-traces")]
    if let Some(path) = std::env::var_os("D2_RS_TRACE_RASTER_SVG") {
        std::fs::write(path, &svg_for_native_raster)
            .map_err(|error| ExportError::Png(format!("diagnostic SVG capture: {error}")))?;
    }
    let exact_dash_parameters = collect_exact_dash_parameters(&svg_for_native_raster);
    // D2 v0.9.0's native scene renderer paints arrowhead geometry directly;
    // it does not clip strokes that extend beyond an SVG marker's viewBox.
    // Allow the generated D2 markers to overflow here so native raster output
    // preserves that geometry. The exported SVG remains unchanged.
    svg_for_native_raster = configure_d2_markers_for_native_raster(&svg_for_native_raster);
    let mut options = resvg::usvg::Options {
        font_family: "Source Sans Pro".to_owned(),
        ..resvg::usvg::Options::default()
    };
    // d2svgimport/image.go accepts only embedded data rasters. Never let
    // usvg's default file resolver bypass the imported-asset admission path,
    // including when it recursively parses an embedded SVG.
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    load_bundled_fonts_for_primary(&mut options, appendix_mono, sketch);
    if !custom_fonts.regular.is_empty() || !custom_fonts.mono.is_empty() {
        svg_for_native_raster = configure_custom_raster_fonts(
            &svg_for_native_raster,
            &custom_fonts,
            &mut options,
            appendix_mono,
        )?;
    }
    load_cached_system_fonts(options.fontdb_mut());
    let emoji_resolver =
        std::sync::Arc::new(std::sync::Mutex::new(bundled_emoji::Resolver::default()));
    // usvg stops searching when the first missing character has no fallback.
    // Load the bundled face independently so a later emoji can still use it,
    // as d2scenebuild resolves the entire scene bucket before shaping.
    // Avoid a second XML parse for ordinary ASCII scenes; numeric text
    // references can still introduce an emoji after XML decoding.
    if let Some(value) = (svg_for_native_raster.contains("&#")
        || svg_for_native_raster
            .chars()
            .any(|value| !value.is_ascii() && bundled_emoji::covers(value)))
    .then(|| {
        roxmltree::Document::parse(&svg_for_native_raster)
            .ok()
            .and_then(|document| {
                document
                    .descendants()
                    .filter(|node| {
                        node.is_text()
                            && node
                                .ancestors()
                                .any(|ancestor| ancestor.has_tag_name("text"))
                            && !node
                                .ancestors()
                                .any(|ancestor| ancestor.has_tag_name("style"))
                    })
                    .filter_map(|node| node.text())
                    .flat_map(str::chars)
                    .find(|value| !value.is_ascii() && bundled_emoji::covers(*value))
            })
    })
    .flatten()
    {
        emoji_resolver
            .lock()
            .expect("emoji fallback resolver")
            .select(value, &[], &mut options.fontdb);
    }
    let fallback_emoji_resolver = std::sync::Arc::clone(&emoji_resolver);
    options.font_resolver.select_fallback = Box::new(move |value, excluded, database| {
        fallback_emoji_resolver
            .lock()
            .expect("emoji fallback resolver")
            .select(value, excluded, database)
            .or_else(|| font_fallback::select_fallback(value, excluded, database))
    });
    if let std::borrow::Cow::Owned(svg) =
        svg_precision::separate_embedded_svg_attributes(&svg_for_native_raster)
    {
        svg_for_native_raster = svg;
    }
    let (tagged_svg, mut svg_image_paths) = svg_precision::tag_embedded_svg_images_with_assets(
        &svg_for_native_raster,
        Some(imported_assets),
    );
    if let std::borrow::Cow::Owned(svg) = tagged_svg {
        svg_for_native_raster = svg;
    }
    svg_image_paths.set_prepared_jpegs(prepared_jpegs.by_asset);
    #[cfg(feature = "diagnostic-traces")]
    if let Ok(path) = std::env::var("D2_RS_TRACE_RASTER_SVG") {
        let _ = std::fs::write(path, svg_for_native_raster.as_bytes());
    }
    let tree = resvg::usvg::Tree::from_str(&svg_for_native_raster, &options)
        .map_err(|error| ExportError::Png(format!("invalid SVG: {error}")))?;
    // d2raster/text.go draws prepared text, without keeping its serialized
    // input. Large code blocks have thousands of native text runs. Their
    // SVG is needed after parsing only to materialize raster handoffs if the
    // native renderer declines; otherwise fallback draws this same tree.
    let svg_for_fallback = (!asset_handoffs.is_empty() || svg_image_paths.has_native_code_texts())
        .then_some(svg_for_native_raster);
    if let Some(error) = emoji_resolver
        .lock()
        .expect("emoji fallback resolver")
        .error
        .clone()
    {
        return Err(ExportError::Png(format!("font fallback: {error}")));
    }
    let [logical_width, logical_height] = svg_image_paths.logical_size(tree.size());
    if gif_frame {
        validate_gif_frame_dimensions(logical_width, logical_height)?;
    }
    if !gif_frame {
        // d2raster.prepare rejects oversized PNG frames before drawing any
        // bands. Full scene admission still determines the final error in
        // export_with_sketch, after the renderer has released its memory.
        validate_raster_frame_dimensions(
            logical_width,
            logical_height,
            pixel_scale,
            paged_max_pixels.unwrap_or(32768 * 32768),
        )?;
    }
    let width = svg_precision::scaled_dimension(logical_width, pixel_scale)
        .ok_or_else(|| ExportError::Png("scaled SVG width is too large".into()))?;
    let height = svg_precision::scaled_dimension(logical_height, pixel_scale)
        .ok_or_else(|| ExportError::Png("scaled SVG height is too large".into()))?;
    // Allocate the fallback pixmap only when the native renderer declines
    // the tree, but keep tiny-skia's Pixmap::new size admission up front.
    let pixmap_admitted = width > 0
        && height > 0
        && i32::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(4))
            .and_then(|row_bytes| {
                ((height - 1) as usize)
                    .checked_mul(row_bytes as usize)?
                    .checked_add(width as usize * 4)
            })
            .is_some();
    if !pixmap_admitted {
        return Err(ExportError::Png("SVG dimensions are too large".into()));
    }
    let background = if transparent_background {
        [0, 0, 0, 0]
    } else {
        [255; 4]
    };
    if let Some(sink) = band_sink {
        if let Some((simple_width, simple_height, pixels)) = scanline::render_simple_bands(
            &tree,
            pixel_scale,
            &raster_sources,
            &svg_image_paths,
            background,
            &exact_dash_parameters,
            sink,
        ) {
            // Empty when the frame went to the sink.
            return Ok((simple_width, simple_height, pixels));
        }
    } else if let Some((simple_width, simple_height, pixels)) =
        scanline::render_simple_with_sources_and_background_and_dash_parameters(
            &tree,
            pixel_scale,
            &raster_sources,
            &svg_image_paths,
            background,
            &exact_dash_parameters,
        )
    {
        return Ok((simple_width, simple_height, pixels));
    }
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("D2_RS_TRACE_RASTER_PIXEL").is_some() {
        eprintln!("d2-rs-raster backend=resvg reason=unsupported-native-scanline-tree");
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| ExportError::Png("SVG dimensions are too large".into()))?;
    if !transparent_background {
        // D2's ordinary raster path uses an opaque white canvas before
        // evaluating blend groups. The paged PDF path instead gives FPDF a
        // transparent board image and paints the page fill behind it.
        pixmap.fill(resvg::tiny_skia::Color::WHITE);
    }
    // resvg decodes images itself: give it the assets' PNG encodings in
    // place of the native sampler's handoff placeholders.
    let materialized;
    let tree = if asset_handoffs.is_empty() && !svg_image_paths.has_native_code_texts() {
        &tree
    } else {
        let svg = svg_image_paths
            .materialize_code_texts(svg_for_fallback.as_deref().expect("native handoff SVG"));
        materialized = resvg::usvg::Tree::from_str(&asset_handoffs.materialize(&svg), &options)
            .map_err(|error| ExportError::Png(format!("invalid SVG: {error}")))?;
        &materialized
    };
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(pixel_scale as f32, pixel_scale as f32),
        &mut pixmap.as_mut(),
    );
    Ok((width, height, pixmap.take()))
}

/// Converts GIF resources to their first frame on the logical canvas before
/// handing the SVG to resvg.  resvg's GIF decoder intentionally exposes the
/// frame rectangle, while D2's v0.9.0 image path preserves the logical screen
/// bounds and the frame's left/top offset.  Normalization is used only for
/// native raster exports; semantic SVG output continues to carry the source
/// asset bytes and animation.
fn validate_imported_svg_assets(
    graph: &Graph,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
) -> Result<d2_rs_math::svg_asset_cache::AssetCache, ExportError> {
    let mut nodes = graph.nodes().collect::<Vec<_>>();
    nodes.sort_by_key(|(_, node)| {
        let mut parent = node.parent;
        let mut depth = 0;
        while let Some(id) = parent {
            depth += 1;
            parent = graph.node(id).and_then(|node| node.parent);
        }
        depth
    });
    let mut cache = d2_rs_math::svg_asset_cache::AssetCache::default();
    for source in &options.unavailable_images {
        cache.resolve_unavailable(source);
    }
    cache.document.budget.maximum = [1 << 20, 4096, 8192, 1 << 20, 65536, 2048, 1024, 1024];
    let import_limits = [65536, 32, 256, 512, 65536, 4096, 128, 8, 64];
    for (id, node) in nodes {
        let Some(style) = styles.get(&id) else {
            continue;
        };
        let Some(icon) = style.icon.as_deref() else {
            continue;
        };
        let opacity = style
            .opacity
            .as_deref()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(1.0);
        if opacity == 0.0
            && !matches!(
                node.shape,
                ShapeKind::Image | ShapeKind::Class | ShapeKind::SqlTable
            )
        {
            continue;
        }
        if cache.by_source.contains_key(icon) {
            continue;
        }
        let decoded;
        let bytes = if let Some(bytes) = options.asset_data.get(icon) {
            bytes.as_slice()
        } else if let Some(bytes) = svg_import::svg_data_uri(icon) {
            decoded = bytes;
            decoded.as_slice()
        } else {
            continue;
        };
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            || bytes.starts_with(&[0xff, 0xd8, 0xff])
            || bytes.starts_with(b"GIF8")
            || bytes.starts_with(b"RIFF")
        {
            continue;
        }
        let prepared = svg_asset_codec::prepare(
            bytes,
            svg_asset_codec::declared_svg(icon),
            svg_asset_codec::CLI_SVG_LIMIT,
        )
        .map_err(|error| {
            ExportError::Render(format!(
                "scene: shape {:?} image asset: imageasset: probe {:?}: {error}",
                node.external_id,
                svg_import::redact_source(icon)
            ))
        })?;
        let Ok(source) = std::str::from_utf8(&prepared) else {
            continue;
        };
        if !svg_asset_codec::declared_svg(icon) && !source.contains("<svg") {
            continue;
        }
        cache
            .resolve_svg_labeled(
                &format!(
                    "shape {}",
                    d2_rs_core::quote_go_bytes(node.external_id.as_bytes())
                ),
                icon,
                &svg_import::redact_source(icon),
                "image/svg+xml",
                &prepared,
                import_limits,
                None,
            )
            .map_err(ExportError::Render)?;
    }
    Ok(cache)
}

struct PreparedJpegs<'a> {
    by_asset: BTreeMap<String, Vec<u8>>,
    by_source: BTreeMap<String, String>,
    used_sources: std::collections::BTreeSet<&'a str>,
}
fn prepare_jpeg_assets<'a>(
    graph: &'a Graph,
    styles: &'a BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    cache: &d2_rs_math::svg_asset_cache::AssetCache,
) -> Result<PreparedJpegs<'a>, ExportError> {
    use d2_rs_math::svg_image::RasterAsset;
    use std::borrow::Cow;
    let mut assets = BTreeMap::<String, Cow<'_, RasterAsset>>::new();
    for (id, asset) in &cache.document.resources {
        if let Some(asset) = asset {
            assets.insert(id.clone(), Cow::Borrowed(asset));
        }
    }
    let mut by_source = BTreeMap::new();
    let mut used_sources = std::collections::BTreeSet::new();
    if let Some(source) = graph
        .legend
        .as_ref()
        .and_then(|legend| legend.source_graph.as_ref())
    {
        used_sources.extend(
            source
                .styles
                .values()
                .filter_map(|style| style.icon.as_deref()),
        );
    }
    for (id, node) in graph.nodes() {
        let Some(style) = styles.get(&id) else {
            continue;
        };
        let Some(icon) = style.icon.as_deref() else {
            continue;
        };
        let opacity = style
            .opacity
            .as_deref()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(1.);
        if opacity == 0.
            && !matches!(
                node.shape,
                ShapeKind::Image | ShapeKind::Class | ShapeKind::SqlTable
            )
        {
            continue;
        }
        used_sources.insert(icon);
        let data = if let Some(data) = options.asset_data.get(icon) {
            Cow::Borrowed(data.as_slice())
        } else if let Some(payload) = icon
            .strip_prefix("data:image/png;base64,")
            .or_else(|| icon.strip_prefix("data:image/jpeg;base64,"))
            .or_else(|| icon.strip_prefix("data:image/gif;base64,"))
        {
            use base64::Engine as _;
            Cow::Owned(
                base64::engine::general_purpose::STANDARD
                    .decode(payload)
                    .map_err(|e| ExportError::Render(format!("invalid base64 data URI: {e}")))?,
            )
        } else {
            continue;
        };
        let data = data.as_ref();
        let format = raster_asset_validation::signature(data);
        if format != "jpeg" && format != "gif" && (format != "png" || is_apng(data)) {
            continue;
        }
        let mime = if format == "jpeg" {
            "image/jpeg"
        } else if format == "gif" {
            "image/gif"
        } else {
            "image/png"
        };
        if by_source.contains_key(icon) {
            continue;
        }
        let digest = d2_rs_math::svg_asset_cache::raster_content_digest(mime, data, None)
            .map_err(ExportError::Render)?;
        let id = format!("image:{digest}");
        by_source.insert(icon.to_owned(), id.clone());
        if assets.contains_key(&id) {
            continue;
        }
        let config = (if format == "jpeg" {
            d2_rs_math::svg_image_config::jpeg_config(data, None)
        } else if format == "gif" {
            d2_rs_math::svg_image_config::gif_config(data, None)
        } else {
            d2_rs_math::svg_image_config::static_config(data, format, None)
        })
        .map_err(|e| {
            ExportError::Render(format!(
                "d2raster: raster asset {} has malformed {} data: {e}",
                d2_rs_core::quote_go_bytes(id.as_bytes()),
                format.to_uppercase()
            ))
        })?;
        assets.insert(
            id,
            Cow::Owned(RasterAsset {
                mime: mime.into(),
                data: data.to_vec(),
                width: config.width,
                height: config.height,
                decoded_bytes: config.decoded_bytes,
            }),
        );
    }
    // Source preflight sorts document resource identities and charges each
    // distinct raster once; aliases and repeated image uses share decoding.
    let mut used = 0i64;
    let mut by_asset = BTreeMap::new();
    for (id, asset) in assets {
        let remaining = (512i64 << 20) - used;
        if asset.mime == "image/jpeg" {
            let (bytes, charge) =
                raster_asset_validation::prepare_jpeg(&id, &asset, remaining, None)
                    .map_err(ExportError::Render)?;
            by_asset.insert(id, bytes);
            used += charge;
        } else if asset.mime == "image/gif" {
            let validation = raster_asset_validation::validate_static(&id, &asset, remaining, None)
                .map_err(ExportError::Render)?;
            gif_pixels::validate(&asset.data).map_err(|e| {
                ExportError::Render(format!(
                    "d2raster: raster asset {} has malformed GIF pixel data: {e}",
                    d2_rs_core::quote_go_bytes(id.as_bytes())
                ))
            })?;
            let normalized = first_gif_frame_png(&asset.data).map_err(|e| {
                ExportError::Render(format!(
                    "d2raster: raster asset {} has malformed GIF pixel data: {e}",
                    d2_rs_core::quote_go_bytes(id.as_bytes())
                ))
            })?;
            by_asset.insert(id, normalized);
            used += validation.decoded_bytes;
        } else if asset.mime == "image/png" {
            used += raster_asset_validation::prepare_png(&id, &asset, remaining, None)
                .map_err(ExportError::Render)?;
            by_asset.insert(id, asset.data.clone());
        } else {
            let validation = raster_asset_validation::validate_static(&id, &asset, remaining, None)
                .map_err(ExportError::Render)?;
            used += validation.decoded_bytes;
            if asset.mime == "image/webp" {
                by_asset.insert(id, asset.data.clone());
            }
        }
    }
    Ok(PreparedJpegs {
        by_asset,
        by_source,
        used_sources,
    })
}
fn normalize_raster_assets(
    mut options: RenderOptions,
    prepared: &PreparedJpegs<'_>,
    handoffs: &mut png_image::HandoffScope,
) -> RenderOptions {
    for (locator, bytes) in &mut options.asset_data {
        // RenderOptions can retain assets from other boards. Decode only
        // sources used by this board; imported resources are prepared by ID.
        if !prepared.used_sources.contains(locator.as_str()) {
            continue;
        }
        if svg_asset_codec::declared_svg(locator) {
            if let Ok(std::borrow::Cow::Owned(decoded)) =
                svg_asset_codec::prepare(bytes, true, svg_asset_codec::CLI_SVG_LIMIT)
            {
                *bytes = decoded;
            }
        }
        let admitted = prepared
            .by_source
            .get(locator)
            .and_then(|id| prepared.by_asset.get(id));
        let data = admitted.map_or(bytes.as_slice(), Vec::as_slice);
        if let Some(pixels) = prepare_raster_pixels(data, handoffs) {
            *bytes = pixels;
        }
    }
    // Inline rasters have no entry in asset_data. Populate their aliases too,
    // so the renderer shares pixels with resolved files of the same content.
    // The original resource identity and validation remain unchanged.
    for (locator, id) in &prepared.by_source {
        if !locator.starts_with("data:") {
            continue;
        }
        if let Some(bytes) = prepared.by_asset.get(id) {
            if let Some(pixels) = prepare_raster_pixels(bytes, handoffs) {
                options.asset_data.insert(locator.clone(), pixels);
            }
        }
    }
    options
}

fn prepare_raster_pixels(data: &[u8], handoffs: &mut png_image::HandoffScope) -> Option<Vec<u8>> {
    let png = data.starts_with(b"\x89PNG\r\n\x1a\n");
    let apng = png && is_apng(data);
    if !png && !data.starts_with(b"GIF8") && !is_webp(data) {
        return None;
    }
    handoffs.prepare(data, png && !apng, || {
        if apng {
            png_image::decode_png(&first_apng_frame_png(data).ok()??)
        } else if png {
            png_image::decode_png(data)
        } else if data.starts_with(b"GIF8") {
            png_image::decode_png(&first_gif_frame_png(data).ok()?)
        } else {
            // Preserve x/image/webp's VP8 YCbCr planes or VP8L NRGBA.
            webp_asset(data).ok().flatten()
        }
    })
}

fn is_apng(data: &[u8]) -> bool {
    if data.len() < 8 || data[..8] != [137, 80, 78, 71, 13, 10, 26, 10] {
        return false;
    }
    let mut offset: usize = 8;
    while offset.checked_add(12).is_some_and(|end| end <= data.len()) {
        let length = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        let chunk_end = offset
            .checked_add(12)
            .and_then(|header_end| header_end.checked_add(length));
        let Some(chunk_end) = chunk_end.filter(|end| *end <= data.len()) else {
            return false;
        };
        if &data[offset + 4..offset + 8] == b"acTL" {
            return true;
        }
        offset = chunk_end;
    }
    false
}

/// Returns a PNG-normalized APNG only when an animation control chunk is
/// present. The pinned decoder yields the first raw subframe; it is composited
/// onto the logical canvas using the source offset and blend rule so raster
/// exports do not mistake a subframe rectangle for the image dimensions.
fn first_apng_frame_png(data: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(
        png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    if reader.info().animation_control.is_none() {
        return Ok(None);
    }
    let canvas_width = usize::try_from(reader.info().width)
        .map_err(|_| "APNG canvas width is too large".to_owned())?;
    let canvas_height = usize::try_from(reader.info().height)
        .map_err(|_| "APNG canvas height is too large".to_owned())?;
    if canvas_width == 0 || canvas_height == 0 {
        return Err("APNG logical canvas is empty".into());
    }
    let frame_control = *reader
        .info()
        .frame_control
        .as_ref()
        .ok_or_else(|| "APNG has no first frame control".to_owned())?;
    let buffer_size = reader
        .output_buffer_size()
        .ok_or_else(|| "APNG first frame is too large".to_owned())?;
    let mut frame = vec![0_u8; buffer_size];
    let output_info = reader
        .next_frame(&mut frame)
        .map_err(|error| error.to_string())?;
    if output_info.bit_depth != png::BitDepth::Eight {
        return Err("APNG decoder did not produce 8-bit pixels".into());
    }
    let frame_width = usize::try_from(output_info.width)
        .map_err(|_| "APNG frame width is too large".to_owned())?;
    let frame_height = usize::try_from(output_info.height)
        .map_err(|_| "APNG frame height is too large".to_owned())?;
    let frame_bytes = output_info.buffer_size();
    let rgba = apng_rgba8(&frame[..frame_bytes], output_info.color_type)?;
    let expected = frame_width
        .checked_mul(frame_height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "APNG frame is too large".to_owned())?;
    if rgba.len() != expected {
        return Err("APNG first frame pixel buffer is truncated".into());
    }
    let offset_x = usize::try_from(frame_control.x_offset)
        .map_err(|_| "APNG frame x offset is too large".to_owned())?;
    let offset_y = usize::try_from(frame_control.y_offset)
        .map_err(|_| "APNG frame y offset is too large".to_owned())?;
    let right = offset_x
        .checked_add(frame_width)
        .ok_or_else(|| "APNG frame width overflows the logical canvas".to_owned())?;
    let bottom = offset_y
        .checked_add(frame_height)
        .ok_or_else(|| "APNG frame height overflows the logical canvas".to_owned())?;
    if right > canvas_width || bottom > canvas_height {
        return Err("APNG first frame falls outside the logical canvas".into());
    }
    let canvas_bytes = canvas_width
        .checked_mul(canvas_height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "APNG logical canvas is too large".to_owned())?;
    let mut canvas = vec![0_u8; canvas_bytes];
    for row in 0..frame_height {
        for column in 0..frame_width {
            let source = (row * frame_width + column) * 4;
            let destination = ((offset_y + row) * canvas_width + offset_x + column) * 4;
            match frame_control.blend_op {
                png::BlendOp::Source => {
                    canvas[destination..destination + 4].copy_from_slice(&rgba[source..source + 4]);
                }
                png::BlendOp::Over => apng_over(
                    &mut canvas[destination..destination + 4],
                    &rgba[source..source + 4],
                ),
            }
        }
    }
    let width = u32::try_from(canvas_width).map_err(|_| "APNG width is too large".to_owned())?;
    let height = u32::try_from(canvas_height).map_err(|_| "APNG height is too large".to_owned())?;
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&canvas)
        .map_err(|error| error.to_string())?;
    drop(writer);
    Ok(Some(output))
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

fn apng_over(destination: &mut [u8], source: &[u8]) {
    let source_alpha = u32::from(source[3]);
    let destination_alpha = u32::from(destination[3]);
    let inverse = 255_u32.saturating_sub(source_alpha);
    let alpha = source_alpha + (destination_alpha * inverse + 127) / 255;
    if alpha == 0 {
        destination.fill(0);
        return;
    }
    for channel in 0..3 {
        let value = u32::from(source[channel]) * source_alpha
            + (u32::from(destination[channel]) * destination_alpha * inverse + 127) / 255;
        destination[channel] = u8::try_from((value + alpha / 2) / alpha).unwrap_or(255);
    }
    destination[3] = u8::try_from(alpha).unwrap_or(255);
}

fn is_webp(data: &[u8]) -> bool {
    data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP"
}

/// Returns a PNG-normalized WebP only when the source is animated. The pinned
/// decoder composites the first ANMF rectangle onto the logical canvas and
/// preserves alpha; static WebP remains in its original encoding so SVG
/// output and non-raster callers do not pay a decode/re-encode cost.
#[cfg(test)]
fn first_webp_frame_png(data: &[u8]) -> Result<Option<Vec<u8>>, String> {
    webp_frame_png(data, true)
}

/// A static or animated WebP as the sampler's asset: a lossless image as
/// straight eight-bit RGBA, a lossy image as its YCbCr planes
/// (`owned_webp_asset`). internal/rasterimage DecodeFirst decodes an animated
/// container's first ANMF frame as that standalone static WebP and, unless it
/// fills the logical canvas, composites it there (compositeFrame).
fn webp_asset(data: &[u8]) -> Result<Option<png_image::Asset>, String> {
    let plan = d2_rs_math::webp_first_frame::inspect(data, true, None)?;
    if !plan.animated {
        return scanline::owned_webp_asset(data);
    }
    let Some(frame) = scanline::owned_webp_asset(&plan.first_frame)? else {
        return Ok(None);
    };
    let (canvas_width, canvas_height) = plan.canvas.unwrap_or_default();
    Ok(frame.on_canvas(
        canvas_width,
        canvas_height,
        plan.frame.0,
        plan.frame.1,
        plan.source_blend,
    ))
}

#[cfg(test)]
fn webp_frame_png(data: &[u8], animated_only: bool) -> Result<Option<Vec<u8>>, String> {
    let Some((width, height, rgba)) = webp_frame_rgba(data, animated_only)? else {
        return Ok(None);
    };
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(rgba.as_flattened())
        .map_err(|error| error.to_string())?;
    drop(writer);
    Ok(Some(output))
}

/// The first WebP frame as straight eight-bit RGBA (x/image/webp NRGBA).
fn webp_frame_rgba(
    data: &[u8],
    animated_only: bool,
) -> Result<Option<(u32, u32, Vec<[u8; 4]>)>, String> {
    let mut decoder =
        WebPDecoder::new(BufReader::new(Cursor::new(data))).map_err(|error| error.to_string())?;
    if animated_only && !decoder.is_animated() {
        return Ok(None);
    }
    let (width, height) = decoder.dimensions();
    let output_size = decoder
        .output_buffer_size()
        .ok_or_else(|| "animated WebP output is too large".to_owned())?;
    let mut decoded = vec![0_u8; output_size];
    decoder
        .read_image(&mut decoded)
        .map_err(|error| error.to_string())?;
    let rgba = if decoder.has_alpha() {
        decoded
            .chunks_exact(4)
            .map(|pixel| [pixel[0], pixel[1], pixel[2], pixel[3]])
            .collect()
    } else {
        decoded
            .chunks_exact(3)
            .map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect()
    };
    Ok(Some((width, height, rgba)))
}

fn first_gif_frame_png(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut decode_options = gif::DecodeOptions::new();
    decode_options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = decode_options
        .read_info(Cursor::new(data))
        .map_err(|error| error.to_string())?;
    let width = usize::from(decoder.width());
    let height = usize::from(decoder.height());
    if width == 0 || height == 0 {
        return Err("GIF logical canvas is empty".into());
    }
    let frame = decoder
        .read_next_frame()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "GIF has no image frames".to_owned())?;
    let frame_left = usize::from(frame.left);
    let frame_top = usize::from(frame.top);
    let frame_width = usize::from(frame.width);
    let frame_height = usize::from(frame.height);
    let frame_right = frame_left
        .checked_add(frame_width)
        .ok_or_else(|| "GIF frame width overflows the logical canvas".to_owned())?;
    let frame_bottom = frame_top
        .checked_add(frame_height)
        .ok_or_else(|| "GIF frame height overflows the logical canvas".to_owned())?;
    if frame_right > width || frame_bottom > height {
        return Err("GIF first frame falls outside the logical canvas".into());
    }
    let expected_bytes = frame_width
        .checked_mul(frame_height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "GIF frame is too large".to_owned())?;
    if frame.buffer.len() < expected_bytes {
        return Err("GIF first frame is truncated".into());
    }
    let canvas_bytes = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "GIF logical canvas is too large".to_owned())?;
    let mut rgba = vec![0_u8; canvas_bytes];
    for row in 0..frame_height {
        let source_start = row * frame_width * 4;
        let destination_start = ((frame_top + row) * width + frame_left) * 4;
        let row_bytes = frame_width * 4;
        rgba[destination_start..destination_start + row_bytes]
            .copy_from_slice(&frame.buffer[source_start..source_start + row_bytes]);
    }
    let width_u32 = u32::try_from(width).map_err(|_| "GIF width is too large".to_owned())?;
    let height_u32 = u32::try_from(height).map_err(|_| "GIF height is too large".to_owned())?;
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width_u32, height_u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&rgba)
        .map_err(|error| error.to_string())?;
    drop(writer);
    Ok(output)
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

fn strip_embedded_font_faces(svg: &str) -> String {
    let marker = "@font-face{";
    let mut output = String::with_capacity(svg.len());
    let mut cursor = 0;
    while let Some(relative_start) = svg[cursor..].find(marker) {
        let start = cursor + relative_start;
        output.push_str(&svg[cursor..start]);
        let body_start = start + marker.len();
        let Some(relative_end) = svg[body_start..].find('}') else {
            output.push_str(&svg[start..]);
            return output;
        };
        cursor = body_start + relative_end + 1;
    }
    output.push_str(&svg[cursor..]);
    output
}

fn emulate_go_synthetic_italic(svg: &str) -> String {
    let mut output = String::with_capacity(svg.len());
    let mut cursor = 0;
    while let Some(relative_start) = svg[cursor..].find("<text") {
        let start = cursor + relative_start;
        output.push_str(&svg[cursor..start]);
        let Some(relative_end) = svg[start..].find('>') else {
            output.push_str(&svg[start..]);
            return output;
        };
        let end = start + relative_end + 1;
        let mut tag = svg[start..end].to_owned();
        // Native Markdown synthetic bold uses a scene f64 stroke width.
        // Give each retained text node a unique identity before usvg reduces
        // its paint operands to f32; the precision reader binds by this ID.
        if tag.contains("class=\"md-text ")
            && svg_attribute(&tag, "stroke-width")
                .and_then(|width| width.parse::<f64>().ok())
                .is_some_and(|width| width.is_finite() && width > 0.0)
        {
            let marker = format!("d2rs-md-stroke-{start}");
            if let Some(existing_id) = svg_attribute(&tag, "id") {
                replace_svg_attribute(&mut tag, "id", &format!("{marker}--{existing_id}"));
            } else {
                tag.insert_str("<text".len(), &format!(" id=\"{marker}\""));
            }
        }
        if (tag.contains("class=\"md-text text-bold") || tag.contains("class=\"md-text text-mono"))
            && tag.contains("font-style=\"italic\"")
        {
            let x = svg_attribute(&tag, "x").unwrap_or_else(|| "0".to_owned());
            let y = svg_attribute(&tag, "y").unwrap_or_else(|| "0".to_owned());
            // Keep the source SVG unchanged, but carry the exact scene pivot
            // into the native raster tree. The upstream scene builder applies
            // this synthetic axis in f64; usvg otherwise composes the SVG
            // transform in f32 before the scanline path sees it.
            let marker = x
                .parse::<f64>()
                .ok()
                .zip(y.parse::<f64>().ok())
                .map(|(x, y)| {
                    format!(
                        "d2rs-synthetic-italic-{:016x}-{:016x}",
                        x.to_bits(),
                        y.to_bits()
                    )
                });
            tag = tag.replace(" font-style=\"italic\"", "");
            let transform =
                format!(" transform=\"translate({x} {y}) skewX(-12) translate(-{x} -{y})\"");
            if let Some(marker) = marker {
                if let Some(existing_id) = svg_attribute(&tag, "id") {
                    replace_svg_attribute(&mut tag, "id", &format!("{marker}--{existing_id}"));
                } else {
                    tag.insert_str("<text".len(), &format!(" id=\"{marker}\""));
                }
            }
            if let Some(index) = tag.find(" xml:space=") {
                tag.insert_str(index, &transform);
            } else if let Some(index) = tag.rfind("/>") {
                tag.insert_str(index, &transform);
            } else {
                tag.insert_str(tag.len() - 1, &transform);
            }
        }
        output.push_str(&tag);
        cursor = end;
    }
    output.push_str(&svg[cursor..]);
    output
}

fn svg_attribute(tag: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=\"");
    let start = tag.find(&prefix)? + prefix.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

fn replace_svg_attribute(tag: &mut String, name: &str, value: &str) {
    let prefix = format!(" {name}=\"");
    let Some(start) = tag.find(&prefix).map(|start| start + prefix.len()) else {
        return;
    };
    let Some(end) = tag[start..].find('"').map(|end| start + end) else {
        return;
    };
    tag.replace_range(start..end, value);
}

fn reset_animated_dash_offsets(svg: &str) -> String {
    set_animated_dash_offsets(svg, None)
}

fn configure_d2_markers_for_native_raster(svg: &str) -> String {
    let mut output = String::with_capacity(svg.len());
    let mut cursor = 0;
    while let Some(relative_start) = svg[cursor..].find("<marker") {
        let start = cursor + relative_start;
        output.push_str(&svg[cursor..start]);
        let Some(relative_end) = svg[start..].find('>') else {
            output.push_str(&svg[start..]);
            return output;
        };
        let end = start + relative_end + 1;
        let mut tag = svg[start..end].to_owned();
        if svg_attribute(&tag, "id").is_some_and(|id| id.starts_with("mk-d2-")) {
            let Some(relative_close) = svg[end..].find("</marker>") else {
                output.push_str(&svg[start..]);
                return output;
            };
            let close = end + relative_close;
            if svg_attribute(&tag, "overflow").is_some() {
                replace_svg_attribute(&mut tag, "overflow", "visible");
            } else if let Some(index) = tag.rfind("/>") {
                tag.insert_str(index, " overflow=\"visible\"");
            } else {
                tag.insert_str(tag.len() - 1, " overflow=\"visible\"");
            }
            output.push_str(&tag);
            output.push_str(&set_marker_stroke_styles(&svg[end..close]));
            output.push_str("</marker>");
            cursor = close + "</marker>".len();
        } else {
            output.push_str(&tag);
            cursor = end;
        }
    }
    output.push_str(&svg[cursor..]);
    output
}

fn set_marker_stroke_styles(content: &str) -> String {
    let mut output = String::with_capacity(content.len());
    let mut cursor = 0;
    loop {
        let path_start = content[cursor..].find("<path");
        let circle_start = content[cursor..].find("<circle");
        let relative_start = match (path_start, circle_start) {
            (Some(path), Some(circle)) => path.min(circle),
            (Some(path), None) => path,
            (None, Some(circle)) => circle,
            (None, None) => break,
        };
        let start = cursor + relative_start;
        output.push_str(&content[cursor..start]);
        let Some(relative_end) = content[start..].find('>') else {
            output.push_str(&content[start..]);
            return output;
        };
        let end = start + relative_end + 1;
        let mut tag = content[start..end].to_owned();
        let style_value = svg_attribute(&tag, "style")
            .map(|style| format!("{style};stroke-linecap:butt;stroke-linejoin:miter"))
            .unwrap_or_else(|| "stroke-linecap:butt;stroke-linejoin:miter".to_owned());
        if svg_attribute(&tag, "style").is_some() {
            replace_svg_attribute(&mut tag, "style", &style_value);
        } else if let Some(index) = tag.rfind("/>") {
            tag.insert_str(index, &format!(" style=\"{style_value}\""));
        } else {
            tag.insert_str(tag.len() - 1, &format!(" style=\"{style_value}\""));
        }
        output.push_str(&tag);
        cursor = end;
    }
    output.push_str(&content[cursor..]);
    output
}

/// Materializes the CSS dash animation at a deterministic raster timestamp.
/// Resvg does not evaluate CSS animations, while D2's native GIF renderer
/// samples them at 30 fps. The source SVG stores the final dash offset in the
/// presentation attribute and defines a `from: 0` keyframe, so linear
/// interpolation over the declared duration reproduces the sampled property.
fn set_animated_dash_offsets(svg: &str, animation_time_ns: Option<i64>) -> String {
    let mut output = String::with_capacity(svg.len());
    let mut cursor = 0;
    while let Some(relative_start) = svg[cursor..].find("<path") {
        let start = cursor + relative_start;
        output.push_str(&svg[cursor..start]);
        let Some(relative_end) = svg[start..].find(">") else {
            output.push_str(&svg[start..]);
            return output;
        };
        let end = start + relative_end + 1;
        let mut tag = svg[start..end].to_owned();
        if tag.contains("animation=\"dashdraw ")
            && let Some(offset_start) = tag.find("stroke-dashoffset=\"")
        {
            let value_start = offset_start + "stroke-dashoffset=\"".len();
            if let Some(relative_quote) = tag[value_start..].find('"') {
                let value_end = value_start + relative_quote;
                let mut target = tag[value_start..value_end].parse::<f64>().unwrap_or(0.0);
                let exact_parameters = exact_dash_animation_parameters(&tag);
                let reverse = tag
                    .split("animation=\"dashdraw ")
                    .nth(1)
                    .and_then(|s| s.split('"').next())
                    .is_some_and(|s| s.split_whitespace().any(|word| word == "reverse"));
                // Go fuses the source lerp on arm64 but not amd64.
                let interpolate = |progress: f64, target: f64| {
                    let (from, to) = if reverse {
                        (target, 0.0)
                    } else {
                        (0.0, target)
                    };
                    #[cfg(target_arch = "aarch64")]
                    {
                        (to - from).mul_add(progress, from)
                    }
                    #[cfg(not(target_arch = "aarch64"))]
                    {
                        from + (to - from) * progress
                    }
                };
                // The scene keeps the unrounded base offset for both the
                // time-zero raster frame and later animation samples.  The
                // public SVG rounds it to six decimals, so recover the scene
                // value before selecting either endpoint of the track.
                if let Some((_, exact_target, _, _)) = exact_parameters {
                    target = exact_target;
                }
                let value = animation_time_ns
                    .map(|timestamp_ns| match exact_parameters {
                        Some((duration_ns, exact_target, _, _)) if duration_ns > 0 => {
                            // The Go renderer keeps sample and animation times as
                            // integer time.Duration values through modulo, then
                            // converts each operand to float64 for interpolation.
                            // Converting both to seconds first changes the final
                            // dash offset by an ULP for some frames.
                            let local_ns = timestamp_ns % duration_ns;
                            let progress = (local_ns as f64) / (duration_ns as f64);
                            interpolate(progress, exact_target)
                        }
                        Some(_) => 0.0,
                        None => {
                            let serialized_animation = tag
                                .split("animation=\"dashdraw ")
                                .nth(1)
                                .and_then(|rest| rest.split('s').next())
                                .and_then(|duration| duration.parse::<f64>().ok());
                            let Some(animation) = serialized_animation
                                .filter(|animation| animation.is_finite() && *animation > 0.0)
                            else {
                                return 0.0;
                            };
                            let seconds = timestamp_ns as f64 / 1_000_000_000.0;
                            let progress = seconds.rem_euclid(animation) / animation;
                            interpolate(progress, target)
                        }
                    })
                    .unwrap_or(if reverse { target } else { 0.0 });
                if let Some((_, _, dash, gap)) = exact_parameters {
                    for (name, operand) in [
                        ("data-d2-rs-dash-size-bits", dash),
                        ("data-d2-rs-dash-gap-bits", gap),
                        ("data-d2-rs-dash-offset-bits", value),
                    ] {
                        let marker = format!("{name}=\"");
                        let bits = format!("{:016x}", operand.to_bits());
                        if let Some(start) = tag.find(&marker) {
                            let from = start + marker.len();
                            if let Some(end) = tag[from..].find('"') {
                                tag.replace_range(from..from + end, &bits);
                            }
                        } else {
                            tag.insert_str("<path".len(), &format!(" {name}=\"{bits}\""));
                        }
                    }
                }
                let rendered = if value == 0.0 {
                    "0".to_owned()
                } else if exact_parameters.is_some() {
                    format!("{value:.17}")
                } else {
                    format!("{value:.6}")
                };
                let rendered_start =
                    tag.find("stroke-dashoffset=\"").unwrap() + "stroke-dashoffset=\"".len();
                let rendered_end = rendered_start + tag[rendered_start..].find('"').unwrap();
                tag.replace_range(rendered_start..rendered_end, &rendered);
                if animation_time_ns.is_some()
                    && let Some((_, _, dash_size, gap_size)) = exact_parameters
                    && let Some(dash_start) = tag.find("stroke-dasharray=\"")
                {
                    let dash_value_start = dash_start + "stroke-dasharray=\"".len();
                    if let Some(relative_dash_quote) = tag[dash_value_start..].find('"') {
                        let dash_value_end = dash_value_start + relative_dash_quote;
                        let exact_dash = format!("{dash_size:.17},{gap_size:.17}");
                        tag.replace_range(dash_value_start..dash_value_end, &exact_dash);
                    }
                }
                if animation_time_ns.is_some()
                    && let Some((_, _, dash_size, gap_size)) = exact_parameters
                    && !tag.contains(" id=\"")
                {
                    // usvg stores SVG dash values as f32. Preserve the source
                    // scene builder's f64 dash pattern and sampled phase in a
                    // raster-only node id so the scanline kernel can recover
                    // them without changing the public SVG representation.
                    let marker = format!(
                        "d2rs-exact-dash-{:016x}-{:016x}-{:016x}",
                        dash_size.to_bits(),
                        gap_size.to_bits(),
                        value.to_bits()
                    );
                    tag.insert_str("<path".len(), &format!(" id=\"{marker}\""));
                }
            }
        }
        output.push_str(&tag);
        cursor = end;
    }
    output.push_str(&svg[cursor..]);
    output
}

/// Reconstructs the unrounded animated-connection dash period used by D2's
/// scene builder from the serialized path attributes. The public SVG remains
/// six-decimal source output; this helper is used only by the native raster
/// sampler, which must retain the source builder's floating-point values.
fn exact_dash_animation_parameters(tag: &str) -> Option<(i64, f64, f64, f64)> {
    let attribute = |name: &str| {
        let marker = format!("{name}=\"");
        let start = tag.find(&marker)? + marker.len();
        let end = start + tag[start..].find('"')?;
        Some(&tag[start..end])
    };
    let stroke_width = attribute("stroke-width")?.parse::<f64>().ok()?;
    let serialized_dash_size = attribute("stroke-dasharray")?
        .split(',')
        .next()?
        .parse::<f64>()
        .ok()?;
    let operand = |name| {
        attribute(name)
            .and_then(|v| u64::from_str_radix(v, 16).ok())
            .map(f64::from_bits)
            .filter(|v| v.is_finite())
    };
    let dash_size = operand("data-d2-rs-dash-size-bits").unwrap_or(serialized_dash_size);
    if !stroke_width.is_finite() || !dash_size.is_finite() {
        return None;
    }
    // Keep this as separate operations: Go evaluates -0.6*width+10.6 without
    // a fused multiply-add, and the final duration is converted to nanoseconds
    // by time.Duration in the source scene builder.
    let scale = (-0.6_f64 * stroke_width + 10.6).log10() * 0.5 + 0.5;
    let gap_size = operand("data-d2-rs-dash-gap-bits").unwrap_or(scale * dash_size);
    let period = dash_size + gap_size;
    if !scale.is_finite() || !gap_size.is_finite() || !period.is_finite() || gap_size <= 0.0 {
        return None;
    }
    let target = attribute("stroke-dashoffset")?.parse::<f64>().ok()?;
    let direction = target.signum();
    if direction == 0.0 {
        return None;
    }
    // Go converts the floating-point duration to time.Duration, truncating
    // toward zero at nanosecond precision before the rasterizer samples it.
    let duration_ns = gap_size * 0.5 * 1_000_000_000.0;
    if !duration_ns.is_finite() || duration_ns < 1.0 || duration_ns > i64::MAX as f64 {
        return None;
    }
    Some((
        duration_ns as i64,
        direction * 10.0 * period,
        dash_size,
        gap_size,
    ))
}

/// Preserves source scene dash operands by their final primitive identities.
/// usvg narrows dash arrays to f32, including rectangles converted into paths.
/// Animated paths override the static tuple with their sampled exact offset.
fn collect_exact_dash_parameters(svg: &str) -> BTreeMap<String, (f64, f64, f64)> {
    let mut parameters = BTreeMap::new();
    if svg.contains("data-d2-rs-dash-size-bits")
        && let Ok(document) = roxmltree::Document::parse(svg)
    {
        for primitive in document.descendants().filter(|node| {
            node.is_element()
                && matches!(
                    node.tag_name().name(),
                    "path" | "rect" | "circle" | "ellipse" | "line" | "polygon" | "polyline"
                )
                && node.attribute("stroke-dasharray").is_some()
        }) {
            let Some(id) = primitive.attribute("id") else {
                continue;
            };
            let exact = primitive.ancestors().find_map(|owner| {
                let operand = |name| {
                    let bits = u64::from_str_radix(owner.attribute(name)?, 16).ok()?;
                    let value = f64::from_bits(bits);
                    value.is_finite().then_some(value)
                };
                let dash = operand("data-d2-rs-dash-size-bits")?;
                let gap = operand("data-d2-rs-dash-gap-bits")?;
                let offset = operand("data-d2-rs-dash-offset-bits")?;
                (dash >= 0.0 && gap >= 0.0 && dash + gap > 0.0).then_some((dash, gap, offset))
            });
            if let Some(exact) = exact {
                parameters.insert(id.to_owned(), exact);
            }
        }
    }
    let mut cursor = 0;
    while let Some(relative_start) = svg[cursor..].find("<path") {
        let start = cursor + relative_start;
        let Some(relative_end) = svg[start..].find('>') else {
            break;
        };
        let end = start + relative_end + 1;
        let tag = &svg[start..end];
        if tag.contains("animation=\"dashdraw ")
            && let (Some(id), Some((_, _, dash, gap)), Some(offset)) = (
                svg_attribute(tag, "id"),
                exact_dash_animation_parameters(tag),
                svg_attribute(tag, "data-d2-rs-dash-offset-bits")
                    .and_then(|v| u64::from_str_radix(&v, 16).ok())
                    .map(f64::from_bits)
                    .or_else(|| {
                        svg_attribute(tag, "stroke-dashoffset")
                            .and_then(|value| value.parse::<f64>().ok())
                    }),
            )
        {
            parameters.insert(id, (dash, gap, offset));
        }
        cursor = end;
    }
    parameters
}

#[cfg(test)]
fn load_bundled_fonts(options: &mut resvg::usvg::Options) {
    load_bundled_fonts_for_primary(options, false, false);
}

// Raster appendix text is scene text in the primary family. Give its
// retained font aliases precedence over the interactive sketch stylesheet.
fn configure_native_appendix_font_roles(svg: &str) -> String {
    if !svg.contains("class=\"appendix\"") {
        return svg.to_owned();
    }
    let Some(end) = svg.rfind("</svg>") else {
        return svg.to_owned();
    };
    let mut output = svg.to_owned();
    output.insert_str(end, r#"<style>.d2-rs-theme .appendix > text{font-family:"d2-rs-appendix-font-regular" !important;}.d2-rs-theme .appendix-icon > text{font-family:"d2-rs-appendix-font-bold" !important;}</style>"#);
    output
}

/// What `Database::load_system_fonts` adds to a database: the host faces in
/// discovery order, and any generic family the host configuration names.
struct SystemFontScan {
    faces: Vec<resvg::usvg::fontdb::FaceInfo>,
    /// Serif, sans-serif, monospace, cursive and fantasy, when configured.
    families: [Option<String>; 5],
}

/// `database.load_system_fonts()`, with the host scan (font configuration
/// parsing, directory walks and face parsing) done once per process rather
/// than once per rendered board. Appending the scanned faces assigns the same
/// IDs in the same order as a direct load into this database.
fn load_cached_system_fonts(database: &mut resvg::usvg::fontdb::Database) {
    use resvg::usvg::fontdb::{Database, Family};
    static SCAN: std::sync::OnceLock<SystemFontScan> = std::sync::OnceLock::new();
    const UNSET: &str = "\0d2-rs-unset-generic-family";
    let scan = SCAN.get_or_init(|| {
        let mut host = Database::new();
        host.set_serif_family(UNSET);
        host.set_sans_serif_family(UNSET);
        host.set_monospace_family(UNSET);
        host.set_cursive_family(UNSET);
        host.set_fantasy_family(UNSET);
        if let Some(directory) = std::env::var_os("D2RS_SYSTEM_FONT_DIR") {
            // Match the typed scene resolver's explicit discovery root.
            host.load_fonts_dir(directory);
        } else {
            host.load_system_fonts();
        }
        let family = |family: Family<'_>| {
            let name = host.family_name(&family);
            (name != UNSET).then(|| name.to_owned())
        };
        SystemFontScan {
            families: [
                family(Family::Serif),
                family(Family::SansSerif),
                family(Family::Monospace),
                family(Family::Cursive),
                family(Family::Fantasy),
            ],
            faces: host.faces().cloned().collect(),
        }
    });
    // One read-once buffer per host file for this database (and its
    // copies), shared by the file's faces.
    let mut sources = std::collections::BTreeMap::new();
    for face in &scan.faces {
        let mut face = face.clone();
        if let resvg::usvg::fontdb::Source::File(path) = &face.source {
            face.source = sources
                .entry(path.clone())
                .or_insert_with(|| font_fallback::host_font_source(path))
                .clone();
        }
        database.push_face_info(face);
    }
    let [serif, sans_serif, monospace, cursive, fantasy] = &scan.families;
    if let Some(name) = serif {
        database.set_serif_family(name.as_str());
    }
    if let Some(name) = sans_serif {
        database.set_sans_serif_family(name.as_str());
    }
    if let Some(name) = monospace {
        database.set_monospace_family(name.as_str());
    }
    if let Some(name) = cursive {
        database.set_cursive_family(name.as_str());
    }
    if let Some(name) = fantasy {
        database.set_fantasy_family(name.as_str());
    }
}

fn load_bundled_fonts_for_primary(
    options: &mut resvg::usvg::Options,
    primary_mono: bool,
    sketch: bool,
) {
    // Export overrides the primary family for mono themes; sketch supplies
    // HandDrawn only when the primary has not been overridden.
    let appendix_regular: &[u8] = if primary_mono {
        d2_rs_text::assets::SOURCE_CODE_PRO_REGULAR_TTF
    } else if sketch {
        d2_rs_text::assets::FUZZY_BUBBLES_REGULAR_TTF
    } else {
        d2_rs_text::assets::SOURCE_SANS_PRO_REGULAR_TTF
    };
    let appendix_bold: &[u8] = if primary_mono {
        d2_rs_text::assets::SOURCE_CODE_PRO_BOLD_TTF
    } else if sketch {
        d2_rs_text::assets::FUZZY_BUBBLES_BOLD_TTF
    } else {
        d2_rs_text::assets::SOURCE_SANS_PRO_BOLD_TTF
    };
    let faces = [
        ("d2-rs-appendix-font-regular", appendix_regular),
        ("d2-rs-appendix-font-bold", appendix_bold),
        (
            "d2-rs-sketch-font-regular",
            d2_rs_text::assets::FUZZY_BUBBLES_REGULAR_TTF,
        ),
        (
            "d2-rs-sketch-font-bold",
            d2_rs_text::assets::FUZZY_BUBBLES_BOLD_TTF,
        ),
        (
            "d2-rs-font-regular",
            d2_rs_text::assets::SOURCE_SANS_PRO_REGULAR_TTF,
        ),
        (
            "d2-rs-font-semibold",
            d2_rs_text::assets::SOURCE_SANS_PRO_SEMIBOLD_TTF,
        ),
        (
            "d2-rs-font-bold",
            d2_rs_text::assets::SOURCE_SANS_PRO_BOLD_TTF,
        ),
        (
            "d2-rs-font-italic",
            d2_rs_text::assets::SOURCE_SANS_PRO_ITALIC_TTF,
        ),
        (
            "d2-rs-font-mono-regular",
            d2_rs_text::assets::SOURCE_CODE_PRO_REGULAR_TTF,
        ),
        (
            "d2-rs-font-mono-semibold",
            d2_rs_text::assets::SOURCE_CODE_PRO_SEMIBOLD_TTF,
        ),
        (
            "d2-rs-font-mono-bold",
            d2_rs_text::assets::SOURCE_CODE_PRO_BOLD_TTF,
        ),
        (
            "d2-rs-font-mono-italic",
            d2_rs_text::assets::SOURCE_CODE_PRO_ITALIC_TTF,
        ),
    ];
    for (alias, data) in faces {
        // The embedded bytes are static: share them rather than copying every
        // face into the heap for each rendered board.
        let ids = options
            .fontdb_mut()
            .load_font_source(resvg::usvg::fontdb::Source::Binary(std::sync::Arc::new(
                data,
            )));
        for id in ids {
            let Some(mut face) = options.fontdb.face(id).cloned() else {
                continue;
            };
            face.families.push((
                alias.to_owned(),
                resvg::usvg::fontdb::Language::English_UnitedStates,
            ));
            options.fontdb_mut().push_face_info(face);
        }
    }
}

// d2scenebuild builder.font retains the selected family's original face,
// independently of its internal family/weight names. AddFontFamily fills
// missing custom styles with SourceSansPro, including for a mono override.
fn configure_custom_raster_fonts(
    svg: &str,
    fonts: &d2_rs_text::d2fonts::CustomFonts,
    options: &mut resvg::usvg::Options,
    primary_mono: bool,
) -> Result<String, ExportError> {
    use d2_rs_text::d2fonts::{FontFamily, FontStyle, font_face};
    use resvg::usvg::fontdb::{Language, Source, Style, Weight};
    let roles = [
        (FontStyle::Regular, "regular", 400, Style::Normal),
        (FontStyle::Semibold, "semibold", 600, Style::Normal),
        (FontStyle::Bold, "bold", 700, Style::Normal),
        (FontStyle::Italic, "italic", 400, Style::Italic),
    ];
    for (mono, faces) in [(false, &fonts.regular), (true, &fonts.mono)] {
        if faces.is_empty() {
            continue;
        }
        // Appendix text and numbered badges use builder.font's default role,
        // hence the diagram's primary family too (d2scenebuild/appendix.go).
        // Their stylesheet aliases take precedence over the text's own family.
        let appendix = mono == primary_mono;
        if appendix {
            let ids: Vec<_> = options
                .fontdb
                .faces()
                .filter(|face| {
                    face.families
                        .iter()
                        .any(|(family, _)| family.starts_with("d2-rs-appendix-font-"))
                })
                .map(|face| face.id)
                .collect();
            for id in ids {
                options.fontdb_mut().remove_face(id);
            }
        }
        for &(role, name, weight, style) in &roles {
            let data = faces
                .get(&role)
                .map(Vec::as_slice)
                .unwrap_or_else(|| font_face(FontFamily::SourceSansPro.font(role)));
            let alias = format!(
                "d2-rs-custom-{}-{name}",
                if mono { "mono" } else { "primary" }
            );
            let ids = options
                .fontdb_mut()
                .load_font_source(Source::Binary(std::sync::Arc::new(data.to_vec())));
            for id in ids {
                let Some(mut face) = options.fontdb.face(id).cloned() else {
                    continue;
                };
                // A CLI role chooses these bytes even when the font's own
                // metadata says regular (e.g. --font-bold regular.ttf).
                face.families = vec![(alias.clone(), Language::English_UnitedStates)];
                if appendix && matches!(role, FontStyle::Regular | FontStyle::Bold) {
                    face.families.push((
                        format!("d2-rs-appendix-font-{name}"),
                        Language::English_UnitedStates,
                    ));
                }
                face.weight = Weight(weight);
                face.style = style;
                options.fontdb_mut().remove_face(id);
                options.fontdb_mut().push_face_info(face);
            }
        }
    }
    let document = roxmltree::Document::parse(svg)
        .map_err(|error| ExportError::Png(format!("invalid SVG: {error}")))?;
    let mut replacements = Vec::new();
    for node in document
        .descendants()
        .filter(|node| node.has_tag_name("text"))
    {
        let class = node.attribute("class").unwrap_or_default();
        let family = node.attribute("font-family").unwrap_or_default();
        let mono = family.contains("Code Pro")
            || family.contains("font-mono")
            || class
                .split_ascii_whitespace()
                .any(|class| class.starts_with("text-mono"))
            || (primary_mono && !class.contains("text-mono"));
        let faces = if mono { &fonts.mono } else { &fonts.regular };
        if faces.is_empty() {
            continue;
        }
        // Leave text inside imported vector assets to their own font rules.
        if node
            .ancestors()
            .any(|ancestor| ancestor.attribute("data-d2-language") == Some("latex"))
        {
            continue;
        }
        let markdown = class
            .split_ascii_whitespace()
            .any(|class| class == "md-text");
        let role = if class.contains("text-mono-semibold") || class.contains("text-semibold") {
            "semibold"
        } else if class.contains("text-mono-bold")
            || class.contains("text-bold")
            || (!markdown && matches!(node.attribute("font-weight"), Some("bold" | "700")))
        {
            "bold"
        } else if class.contains("text-mono-italic")
            || class.contains("text-italic")
            || (!markdown && node.attribute("font-style") == Some("italic"))
        {
            "italic"
        } else {
            "regular"
        };
        let start = node.range().start;
        let end = start + svg[start..].find('>').unwrap();
        let mut tag = svg[start..end].to_owned();
        let alias = format!(
            "d2-rs-custom-{}-{role}",
            if mono { "mono" } else { "primary" }
        );
        let rule = format!("font-family:{alias} !important;");
        if let Some(style) = node.attribute("style") {
            replace_svg_attribute(&mut tag, "style", &format!("{style};{rule}"));
        } else {
            tag.push_str(&format!(" style=\"{rule}\""));
        }
        replacements.push((start..end, tag));
    }
    let mut output = svg.to_owned();
    for (range, tag) in replacements.into_iter().rev() {
        output.replace_range(range, &tag);
    }
    Ok(output)
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
    fn svg_import_preflight_matches_shape_emission_and_svg_boundary() {
        use base64::Engine as _;
        use d2_rs_core::{LayoutOptions, Node, Size};
        use d2_rs_tala::Engine;
        let asset = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><text x="2" y="12">A</text></svg>"#;
        for shape in [
            ShapeKind::Rectangle,
            ShapeKind::Image,
            ShapeKind::Class,
            ShapeKind::SqlTable,
        ] {
            let mut graph = Graph::default();
            let id = graph.add_node(Node {
                shape,
                ..Node::new(
                    "asset",
                    Size {
                        width: 40.0,
                        height: 40.0,
                    },
                )
            });
            let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
            for opacity in ["0", "1"] {
                let styles = BTreeMap::from([(
                    id,
                    SourceNodeStyle {
                        icon: Some("asset.svg".into()),
                        opacity: Some(opacity.into()),
                        ..Default::default()
                    },
                )]);
                let options = RenderOptions {
                    asset_data: BTreeMap::from([("asset.svg".into(), asset.to_vec())]),
                    ..Default::default()
                };
                let actual = super::validate_imported_svg_assets(&graph, &styles, &options);
                let emitted = opacity != "0" || shape != ShapeKind::Rectangle;
                assert_eq!(
                    actual.is_err(),
                    emitted,
                    "shape={shape:?} opacity={opacity}"
                );
                if emitted {
                    assert_eq!(
                        actual.unwrap_err().to_string(),
                        "scene: shape \"asset\" SVG asset: d2svgimport: asset.svg: element <text> has unsupported attribute \"x\""
                    );
                }
                assert!(
                    super::export_with_sketch(
                        &graph,
                        &result,
                        &BTreeMap::new(),
                        &styles,
                        ExportFormat::Svg,
                        options,
                        false
                    )
                    .is_ok()
                );
            }
            let styles = BTreeMap::from([(
                id,
                SourceNodeStyle {
                    icon: Some("asset.svg".into()),
                    ..Default::default()
                },
            )]);
            for format in [
                ExportFormat::Png,
                ExportFormat::Gif,
                ExportFormat::Pdf,
                ExportFormat::Pptx,
            ] {
                let options = RenderOptions {
                    asset_data: BTreeMap::from([("asset.svg".into(), asset.to_vec())]),
                    ..Default::default()
                };
                let actual = super::export_with_sketch(
                    &graph,
                    &result,
                    &BTreeMap::new(),
                    &styles,
                    format,
                    options,
                    false,
                )
                .unwrap_err();
                assert_eq!(
                    actual.to_string(),
                    "scene: shape \"asset\" SVG asset: d2svgimport: asset.svg: element <text> has unsupported attribute \"x\""
                );
            }
            for icon in [
                format!(
                    "data:image/svg+xml;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(asset)
                ),
                format!(
                    "data:image/svg+xml,{}",
                    asset
                        .iter()
                        .map(|byte| format!("%{byte:02X}"))
                        .collect::<String>()
                ),
            ] {
                let styles = BTreeMap::from([(
                    id,
                    SourceNodeStyle {
                        icon: Some(icon),
                        ..Default::default()
                    },
                )]);
                let actual =
                    super::validate_imported_svg_assets(&graph, &styles, &RenderOptions::default())
                        .unwrap_err();
                assert_eq!(
                    actual.to_string(),
                    "scene: shape \"asset\" SVG asset: d2svgimport: data URI: element <text> has unsupported attribute \"x\""
                );
            }
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn sketch_primary_and_mono_labels_resolve_original_go_font_roles() {
        use d2_rs_core::{LayoutOptions, Node, Size};
        use d2_rs_tala::Engine;
        use resvg::usvg::fontdb::{Family, Query, Stretch, Style, Weight};
        use resvg::usvg::{FontFamily, Node as SvgNode};
        use sha2::{Digest, Sha256};
        let controls: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sketch-font-roles-source-v090.json"
        ))
        .unwrap();
        fn fonts(group: &resvg::usvg::Group, output: &mut Vec<resvg::usvg::Font>) {
            for node in group.children() {
                match node {
                    SvgNode::Group(group) => fonts(group, output),
                    SvgNode::Text(text) => {
                        for chunk in text.chunks() {
                            for span in chunk.spans() {
                                output.push(span.font().clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        for control in controls.as_array().unwrap() {
            let primary = control["primary"].as_str().unwrap();
            let mono_theme = primary == "SourceCodePro";
            let sketch = primary != "SourceSansPro";
            let mut graph = Graph::default();
            let node = graph.add_node(Node::new(
                "font",
                Size {
                    width: 140.0,
                    height: 80.0,
                },
            ));
            let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
            let labels = BTreeMap::from([(node, "Agm".to_owned())]);
            let styles = BTreeMap::from([(
                node,
                SourceNodeStyle {
                    font: Some(control["role"].as_str().unwrap().to_owned()),
                    bold: control["bold"].as_bool().unwrap(),
                    italic: control["italic"].as_bool().unwrap(),
                    ..SourceNodeStyle::default()
                },
            )]);
            // This source fixture describes native scene FontRef roles. Public
            // SVG selects concrete Fuzzy faces without synthetic italic.
            let (svg, _) = d2_rs_render::render_svg_with_raster_sources_checked(
                &graph,
                &result,
                &labels,
                &styles,
                RenderOptions {
                    theme_id: mono_theme.then_some(300),
                    ..RenderOptions::default()
                },
                sketch,
            )
            .unwrap();
            let svg = strip_embedded_font_faces(std::str::from_utf8(&svg).unwrap());
            let mut options = resvg::usvg::Options::default();
            load_bundled_fonts_for_primary(&mut options, mono_theme, sketch);
            let tree = resvg::usvg::Tree::from_str(&svg, &options).unwrap();
            let mut resolved = Vec::new();
            fonts(tree.root(), &mut resolved);
            assert_eq!(resolved.len(), 1, "one complete label control {control}");
            let font = &resolved[0];
            let expected_family = match control["family"].as_str().unwrap() {
                "SourceSansPro" => "Source Sans Pro",
                "SourceCodePro" => "Source Code Pro",
                "HandDrawn" => {
                    if control["bold"].as_bool().unwrap() {
                        "d2-rs-sketch-font-bold"
                    } else {
                        "d2-rs-sketch-font-regular"
                    }
                }
                other => panic!("unknown Source family {other}"),
            };
            assert_eq!(
                font.families()[0],
                FontFamily::Named(expected_family.to_owned()),
                "{control}"
            );
            assert_eq!(
                font.weight(),
                control["weight"].as_u64().unwrap() as u16,
                "{control}"
            );
            let expected_style = if control["style"] == "italic" {
                Style::Italic
            } else {
                Style::Normal
            };
            assert_eq!(Style::from(font.style()), expected_style, "{control}");
            let face = options
                .fontdb
                .query(&Query {
                    families: &[Family::Name(expected_family)],
                    weight: Weight(font.weight()),
                    stretch: Stretch::Normal,
                    style: expected_style,
                })
                .unwrap();
            let digest = options
                .fontdb
                .with_face_data(face, |data, _| format!("{:x}", Sha256::digest(data)))
                .unwrap();
            assert_eq!(
                digest,
                control["font_sha256"].as_str().unwrap(),
                "Source font asset {control}"
            );
        }
    }

    #[test]
    fn appendix_aliases_resolve_to_source_faces_and_identical_raster_glyphs() {
        use resvg::usvg::fontdb::{Family, Query, Stretch, Style, Weight};
        let mut options = resvg::usvg::Options::default();
        load_bundled_fonts_for_primary(&mut options, false, false);
        for (alias, weight, text) in [
            ("d2-rs-appendix-font-regular", 400, "ρν metadata"),
            ("d2-rs-appendix-font-bold", 700, "1234567890"),
        ] {
            let query = Query {
                families: &[Family::Name(alias)],
                weight: Weight(weight),
                stretch: Stretch::Normal,
                style: Style::Normal,
            };
            assert!(
                options.fontdb.query(&query).is_some(),
                "registered appendix face: {alias}"
            );
            let paint = |family: &str| {
                let svg = format!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" width="250" height="60"><text x="12.5" y="32.25" font-family="{family}" font-weight="{weight}" font-size="16">{text}</text></svg>"#
                );
                let svg = strip_embedded_font_faces(&svg);
                let tree = resvg::usvg::Tree::from_str(&svg, &options).unwrap();
                scanline::render_simple(&tree, 2).unwrap()
            };
            let expected = paint("Source Sans Pro");
            assert!(expected.2.chunks_exact(4).any(|pixel| pixel[3] != 0));
            assert_eq!(paint(alias), expected);
        }
    }

    #[test]
    fn appendix_raster_uses_primary_family_and_mono_precedes_sketch() {
        use resvg::usvg::fontdb::{Family, Query, Stretch, Style, Weight};
        for (mono, sketch, expected_family) in [
            (false, false, "Source Sans Pro"),
            (true, false, "Source Code Pro"),
            (false, true, "Fuzzy Bubbles"),
            (true, true, "Source Code Pro"),
        ] {
            let mut options = resvg::usvg::Options::default();
            load_bundled_fonts_for_primary(&mut options, mono, sketch);
            for (alias, weight) in [
                ("d2-rs-appendix-font-regular", 400),
                ("d2-rs-appendix-font-bold", 700),
            ] {
                let query = Query {
                    families: &[Family::Name(alias)],
                    weight: Weight(weight),
                    stretch: Stretch::Normal,
                    style: Style::Normal,
                };
                assert!(options.fontdb.query(&query).is_some());
                let paint = |family: &str| {
                    let svg = format!(
                        r#"<svg xmlns="http://www.w3.org/2000/svg" width="250" height="60"><text x="12.5" y="32.25" font-family="{family}" font-weight="{weight}" font-size="16">metadata AVgy 1234567890</text></svg>"#
                    );
                    let tree = resvg::usvg::Tree::from_str(&svg, &options).unwrap();
                    scanline::render_simple(&tree, 2).unwrap()
                };
                let expected = paint(expected_family);
                assert!(expected.2.chunks_exact(4).any(|pixel| pixel[3] != 0));
                assert_eq!(
                    paint(alias),
                    expected,
                    "mono={mono} sketch={sketch} {alias}"
                );
            }
            for (alias, weight) in [
                ("d2-rs-sketch-font-regular", 400),
                ("d2-rs-sketch-font-bold", 700),
            ] {
                let query = Query {
                    families: &[Family::Name(alias)],
                    weight: Weight(weight),
                    stretch: Stretch::Normal,
                    style: Style::Normal,
                };
                let face = options
                    .fontdb
                    .face(options.fontdb.query(&query).unwrap())
                    .unwrap();
                assert!(
                    face.families
                        .iter()
                        .any(|(name, _)| name == "Fuzzy Bubbles")
                );
            }
        }
    }

    #[test]
    fn raster_appendix_primary_roles_override_sketch_css() {
        for mono in [false, true] {
            let mut options = resvg::usvg::Options::default();
            load_bundled_fonts_for_primary(&mut options, mono, true);
            let family = if mono {
                "Source Code Pro"
            } else {
                "Fuzzy Bubbles"
            };
            let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" class="d2-rs-theme d2-rs-sketch" width="250" height="60"><style>.d2-rs-theme.d2-rs-sketch text{font-family:"d2-rs-sketch-font-regular";}.d2-rs-theme.d2-rs-sketch text[font-weight="bold"]{font-family:"d2-rs-sketch-font-bold";}</style><g class="appendix"><text x="12.5" y="32.25" font-size="16">metadata AVgy</text><g class="appendix-icon"><text x="12.5" y="52.25" font-weight="bold" font-size="16">0123456789</text></g></g></svg>"#;
            let prepared = configure_native_appendix_font_roles(svg);
            let expected = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="250" height="60"><text x="12.5" y="32.25" font-family="{family}" font-size="16">metadata AVgy</text><text x="12.5" y="52.25" font-family="{family}" font-weight="bold" font-size="16">0123456789</text></svg>"#
            );
            let paint = |svg: &str| {
                let tree = resvg::usvg::Tree::from_str(svg, &options).unwrap();
                scanline::render_simple(&tree, 2).unwrap()
            };
            assert_eq!(paint(&prepared), paint(&expected), "mono={mono}");
            if mono {
                assert_ne!(paint(svg), paint(&expected));
            }
        }
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
    fn native_raster_uses_scene_geometry_for_generated_d2_markers() {
        let svg = r#"<svg><defs><marker id="mk-d2-arrow" markerWidth="18" markerHeight="18"><circle cx="5.3" cy="9" r="3.3"/><path d="M0 0 L24 0"/></marker><marker id="external-arrow" markerWidth="8" markerHeight="8"><circle cx="2" cy="2" r="1"/><path d="M0 0 L9 0"/></marker></defs></svg>"#;
        let raster_svg = configure_d2_markers_for_native_raster(svg);
        assert!(raster_svg.contains(
            r#"<marker id="mk-d2-arrow" markerWidth="18" markerHeight="18" overflow="visible"><circle cx="5.3" cy="9" r="3.3" style="stroke-linecap:butt;stroke-linejoin:miter"/><path d="M0 0 L24 0" style="stroke-linecap:butt;stroke-linejoin:miter"/></marker>"#
        ));
        assert!(raster_svg.contains(r#"<marker id="external-arrow" markerWidth="8" markerHeight="8"><circle cx="2" cy="2" r="1"/><path d="M0 0 L9 0"/></marker>"#));
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

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn bundled_color_glyphs_keep_font_units_and_clip_inside_the_label() {
        // Source d2raster/colrv1.go applies fontToText and the glyph clip
        // together. A flattened color glyph must not paint its font-unit
        // bounds across the padded canvas. Exercise several paint trees,
        // and require visible color so silently dropping glyphs cannot pass.
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 100.0,
                height: 60.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        for label in ["✅", "😀", "❤️", "1️⃣"] {
            let labels = BTreeMap::from([(node, label.to_owned())]);
            let (width, height, rgba) = rasterize(
                &graph,
                &result,
                &labels,
                &BTreeMap::new(),
                RenderOptions {
                    pad: 64.0,
                    ..RenderOptions::default()
                },
                false,
                1,
            )
            .unwrap();
            let mut colored = 0;
            for (index, pixel) in rgba.chunks_exact(4).enumerate() {
                let x = index as u32 % width;
                let y = index as u32 / width;
                if x < 16 || y < 16 || x >= width - 16 || y >= height - 16 {
                    assert_eq!(pixel, [255; 4], "{label}: canvas padding at {x},{y}");
                }
                colored += usize::from(
                    pixel[0].abs_diff(pixel[1]) > 32 || pixel[1].abs_diff(pixel[2]) > 32,
                );
            }
            assert!(colored > 0, "{label}: color glyph must be visible");
        }
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
    fn native_raster_keeps_embedded_raster_images() {
        // The custom scanline renderer intentionally declines image nodes and
        // lets resvg handle the complete scene.  Keep a real PNG asset here
        // so the resvg raster-images feature cannot regress back to silently
        // dropping bundled assets from PNG/PDF/PPTX output.
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
            shape: ShapeKind::Image,
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
    fn gif_first_frame_normalization_preserves_logical_canvas_offset() {
        let mut gif_bytes = Vec::new();
        let mut encoder = gif::Encoder::new(&mut gif_bytes, 4, 4, &[]).unwrap();
        let mut frame_pixels = [220, 20, 30, 255].repeat(2 * 2);
        let mut frame = gif::Frame::from_rgba_speed(2, 2, &mut frame_pixels, 10);
        frame.left = 1;
        frame.top = 1;
        frame.width = 2;
        frame.height = 2;
        encoder.write_frame(&frame).unwrap();
        drop(encoder);

        let normalized = first_gif_frame_png(&gif_bytes).unwrap();
        let decoder = png::Decoder::new(Cursor::new(normalized));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (4, 4));
        let center = ((1 * 4 + 1) * 4) as usize;
        assert_eq!(&pixels[center..center + 4], &[220, 20, 30, 255]);
        assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn animated_webp_first_frame_normalization_preserves_canvas() {
        let webp = include_bytes!("../tests/fixtures/animated-first-frame.webp");
        let normalized = first_webp_frame_png(webp).unwrap().unwrap();
        let decoder = png::Decoder::new(Cursor::new(normalized));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (2, 2));
        assert!(
            pixels.chunks_exact(4).all(|pixel| {
                pixel[0] > 180 && pixel[1] < 80 && pixel[2] < 80 && pixel[3] == 255
            })
        );
    }

    #[test]
    fn animated_apng_first_frame_normalization_preserves_canvas() {
        let apng = include_bytes!("../tests/fixtures/animated-first-frame.png");
        assert!(is_apng(apng));
        let normalized = first_apng_frame_png(apng).unwrap().unwrap();
        let decoder = png::Decoder::new(Cursor::new(normalized));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (4, 3));
        assert!(
            pixels.chunks_exact(4).all(|pixel| {
                pixel[0] > 180 && pixel[1] < 80 && pixel[2] < 80 && pixel[3] == 255
            })
        );
    }

    #[test]
    fn static_png_assets_are_not_reencoded_as_apng() {
        let mut encoded = Vec::new();
        let mut encoder = png::Encoder::new(&mut encoded, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[220, 20, 30, 255]).unwrap();
        drop(writer);
        assert!(!is_apng(&encoded));
        assert!(first_apng_frame_png(&encoded).unwrap().is_none());
    }

    #[test]
    fn static_webp_assets_are_not_reencoded() {
        let mut encoded = Vec::new();
        image_webp::WebPEncoder::new(&mut encoded)
            .encode(
                &[
                    220, 20, 30, 255, 220, 20, 30, 255, 220, 20, 30, 255, 220, 20, 30, 255,
                ],
                2,
                2,
                image_webp::ColorType::Rgba8,
            )
            .unwrap();
        assert!(is_webp(&encoded));
        assert!(first_webp_frame_png(&encoded).unwrap().is_none());
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
    fn native_raster_resets_only_animated_dash_offsets() {
        let svg = r#"<style>@keyframes dashdraw{from{stroke-dashoffset:0}}</style>
<path stroke-dashoffset="-873.183058" animation="dashdraw 21.159153s linear infinite" />
<path stroke-dashoffset="-4" />"#;
        let output = reset_animated_dash_offsets(svg);
        assert!(output.contains("stroke-dashoffset=\"0\" animation=\"dashdraw"));
        assert!(output.contains("<path stroke-dashoffset=\"-4\" />"));
        assert!(output.contains("@keyframes dashdraw{from{stroke-dashoffset:0}}"));
        let sampled = set_animated_dash_offsets(svg, Some(10_579_576_500));
        assert!(sampled.contains("stroke-dashoffset=\"-436.591529\""));
    }

    #[test]
    fn native_raster_keeps_exact_animated_dash_f64_values() {
        let svg = r#"<path stroke-width="5" stroke-dasharray="45,42.318306"
stroke-dashoffset="-873.183058" animation="dashdraw 21.159153s linear infinite reverse" />"#;
        let initial = reset_animated_dash_offsets(svg);
        assert!(
            initial.contains("stroke-dashoffset=\"-873.18305826317805440\""),
            "{initial}"
        );
        assert!(
            initial.contains("data-d2-rs-dash-offset-bits=\"c08b4976e7402ce6\""),
            "{initial}"
        );
        let sampled = set_animated_dash_offsets(svg, Some(10_579_576_500));
        assert!(sampled.contains("stroke-dasharray=\"45.00000000000000000,42.31830582631780402\""));
        assert!(sampled.contains("id=\"d2rs-exact-dash-"));
    }

    #[test]
    fn native_raster_retains_exact_dash_values_for_existing_path_ids() {
        let svg = r#"<path id="edge-id" stroke-width="5" stroke-dasharray="45,42.318306"
stroke-dashoffset="-873.183058" animation="dashdraw 21.159153s linear infinite" />"#;
        let sampled = set_animated_dash_offsets(svg, Some(33_333_333));
        let parameters = collect_exact_dash_parameters(&sampled);
        assert_eq!(
            parameters.get("edge-id"),
            Some(&(
                45.0,
                f64::from_bits(0x4045_28be_3ecd_14a3),
                f64::from_bits(0xbff6_025f_e18c_8aef),
            ))
        );
    }

    #[test]
    fn raster_ordinary_dash_collector_preserves_exact_rect_and_path_identities() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
          <g data-d2-rs-dash-size-bits="4024000000000000"
             data-d2-rs-dash-gap-bits="4023bb3511f45fb3"
             data-d2-rs-dash-offset-bits="0000000000000000">
            <rect id="ordinary-rect" x="0" y="0" width="786" height="711"
                  stroke-width="2" stroke-dasharray="10,9.865639"/>
            <path id="ordinary-path" d="M0 0L200 0"
                  stroke-width="2" stroke-dasharray="10,9.865639"/>
          </g>
        </svg>"#;
        let parameters = collect_exact_dash_parameters(svg);
        let expected = (10.0, f64::from_bits(0x4023_bb35_11f4_5fb3), 0.0);
        assert_eq!(parameters.get("ordinary-rect"), Some(&expected));
        assert_eq!(parameters.get("ordinary-path"), Some(&expected));
        assert_eq!(parameters.len(), 2);
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

    #[test]
    fn rgba_unpremultiplication_matches_d2raster_integer_rounding() {
        let mut rgba = [10, 39, 40, 202, 6, 20, 0, 128];
        unpremultiply_rgba(&mut rgba);
        assert_eq!(rgba, [12, 49, 50, 202, 11, 39, 0, 128]);
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
            resvg::tiny_skia::Pixmap::decode_png(png)
                .unwrap()
                .data()
                .to_vec()
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

/// Original d2scenebuild.validateShape has a stricter positioned-tooltip
/// admission contract than d2svg.Render, which permits empty tooltip text.
fn validate_positioned_tooltip_scene_fields(
    graph: &Graph,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> Result<(), ExportError> {
    for (id, node) in graph.nodes() {
        let Some(style) = styles.get(&id) else {
            continue;
        };
        let Some(position) = style
            .tooltip_position
            .as_deref()
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let tooltip = style.tooltip.as_deref().unwrap_or("");
        if tooltip.is_empty() {
            return Err(ExportError::Render(format!(
                "scene: shape {:?} field \"tooltipPosition\" has invalid value {}: requires a non-empty tooltip",
                node.external_id, position
            )));
        }
        let labels = BTreeMap::new();
        let one = BTreeMap::from([(id, style.clone())]);
        if let Err(error) = d2_rs_render::validate_markdown_labels(&labels, &one) {
            return Err(ExportError::Render(format!(
                "scene: shape {:?} positioned tooltip Markdown layout: {}",
                node.external_id, error
            )));
        }
    }
    Ok(())
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
    #[test]
    fn all_source_png_controls_match_scene_admission_and_error_context() {
        let inputs: serde_json::Value = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-render"),
            "tests/fixtures/positioned-tooltip-source-v090.json",
        ))
        .unwrap();
        let expected: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/positioned-tooltip-png-admission-source-v090.json"
        ))
        .unwrap();
        assert_eq!(inputs.as_array().unwrap().len(), 480);
        assert_eq!(expected.as_array().unwrap().len(), 480);
        for (input, source) in inputs
            .as_array()
            .unwrap()
            .iter()
            .zip(expected.as_array().unwrap())
        {
            assert_eq!(input["name"], source["name"]);
            let shape = &input["shape"];
            let mut graph = Graph::default();
            let id = graph.add_node(d2_rs_core::Node::new(
                "probe",
                d2_rs_core::Size {
                    width: 160.0,
                    height: 80.0,
                },
            ));
            let field = |key: &str| {
                shape
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            let styles = BTreeMap::from([(
                id,
                SourceNodeStyle {
                    tooltip: field("tooltip"),
                    tooltip_position: field("tooltipPosition"),
                    ..Default::default()
                },
            )]);
            let error = validate_positioned_tooltip_scene_fields(&graph, &styles)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default();
            assert_eq!(
                error,
                source["error"].as_str().unwrap(),
                "{}",
                input["name"]
            );
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod markdown_background_raster_contract {
    use super::*;
    use d2_rs_core::{LayoutOptions, ast::Shared};
    use d2_rs_language::{compiler, parse};
    use sha2::{Digest, Sha256};

    #[test]
    fn filled_markdown_viewport_retains_complete_source_rgba_surface() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-background-raster-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 24);
        for row in rows.as_array().unwrap() {
            let parsed = parse("", row["source"].as_str().unwrap().as_bytes(), None);
            assert!(parsed.error.is_none());
            let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
            // Isolate the raster contract at the Source-measured viewport.
            // Measurement and target handoff are independently checked by
            // the complete 64-row renderer contract and actual CLI captures.
            let viewport = d2_rs_core::Size {
                width: row["shape"]["labelWidth"].as_f64().unwrap(),
                height: row["shape"]["labelHeight"].as_f64().unwrap(),
            };
            let (graph, labels, styles) =
                compiler::lower_graph_with_measurer_and_labels(&compiler, root, &|_: &str| {
                    viewport
                })
                .unwrap();
            let mut layout = d2_rs_tala::Engine
                .layout(&graph, &LayoutOptions::default())
                .unwrap();
            // This gate begins at the original measured/exported target.
            // Native CLI captures separately qualify integrated sizing.
            let node_id = graph.nodes().next().unwrap().0;
            // Including its exported label position, which the renderer
            // reads (markdownShapeLabelTopLeft).
            if let Some(position) = row["shape"]["labelPosition"]
                .as_str()
                .map(d2_rs_core::LabelPosition::from_d2_name)
            {
                layout.node_labels.entry(node_id).or_default().position = position;
            }
            layout.boxes.insert(
                node_id,
                Rect {
                    origin: Point {
                        x: row["shape"]["pos"]["x"].as_f64().unwrap(),
                        y: row["shape"]["pos"]["y"].as_f64().unwrap(),
                    },
                    size: d2_rs_core::Size {
                        width: row["shape"]["width"].as_f64().unwrap(),
                        height: row["shape"]["height"].as_f64().unwrap(),
                    },
                },
            );
            let options = RenderOptions {
                theme_id: row["theme"].as_i64(),
                ..RenderOptions::default()
            };
            let (width, height, rgba) = rasterize_with_animation_time_and_background(
                &graph, &layout, &labels, &styles, options, false, 2, None, false, false, None,
            )
            .unwrap();
            let expected = &row["source_frame"];
            assert_eq!(
                (u64::from(width), u64::from(height)),
                (
                    expected["width"].as_u64().unwrap(),
                    expected["height"].as_u64().unwrap()
                ),
                "{} complete canvas",
                row["name"]
            );
            assert_eq!(rgba.len(), width as usize * height as usize * 4);
            assert!(rgba.chunks_exact(4).all(|p| p[3] == 255));
            let rgb = rgba
                .chunks_exact(4)
                .flat_map(|p| p[..3].iter().copied())
                .collect::<Vec<_>>();
            assert_eq!(
                format!("{:x}", Sha256::digest(&rgb)),
                expected["rgb_sha256"].as_str().unwrap(),
                "{} complete Source surface",
                row["name"]
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
            let view = &row["viewbox"];
            let sx = row["logical_width"].as_f64().unwrap() / view["Width"].as_f64().unwrap();
            let sy = row["logical_height"].as_f64().unwrap() / view["Height"].as_f64().unwrap();
            let expected = row["links"]
                .as_array()
                .unwrap()
                .iter()
                .map(|link| {
                    let b = &link["Box"];
                    DocumentLinkRegion {
                        href: link["URL"].as_str().unwrap().to_owned(),
                        rect: Rect {
                            origin: Point {
                                x: (b["X"].as_f64().unwrap() - view["X"].as_f64().unwrap()) * sx,
                                y: (b["Y"].as_f64().unwrap() - view["Y"].as_f64().unwrap()) * sy,
                            },
                            size: Size {
                                width: b["Width"].as_f64().unwrap() * sx,
                                height: b["Height"].as_f64().unwrap() * sy,
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

#[cfg(all(test, feature = "oracle-fixtures"))]
mod native_import_preflight_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[derive(serde::Deserialize)]
    struct Row {
        input: String,
        source: String,
        error: String,
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn raster_export_preflight_uses_complete_native_import_and_cli_limits() {
        let rows: Vec<Row> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-native-preflight-v090.json"
        ))
        .unwrap();
        for (i, row) in rows.iter().enumerate() {
            let mut graph = Graph::default();
            let id = graph.add_node(d2_rs_core::Node::new(
                "n",
                d2_rs_core::Size {
                    width: 40.,
                    height: 40.,
                },
            ));
            let styles = BTreeMap::from([(
                id,
                SourceNodeStyle {
                    icon: Some(row.source.clone()),
                    ..Default::default()
                },
            )]);
            let options = RenderOptions {
                asset_data: BTreeMap::from([(row.source.clone(), row.input.as_bytes().to_vec())]),
                ..Default::default()
            };
            let result = validate_imported_svg_assets(&graph, &styles, &options);
            assert_eq!(
                result.err().map(|e| e.to_string()).unwrap_or_default(),
                row.error,
                "control {i}"
            );
        }
    }
}

#[doc(hidden)]
pub mod raster_asset_validation;

#[cfg(all(test, feature = "oracle-fixtures"))]
mod jpeg_asset_caller_contract {
    use super::*;
    fn graph(icons: &[&str]) -> (Graph, BTreeMap<NodeId, SourceNodeStyle>) {
        let mut graph = Graph::default();
        let mut styles = BTreeMap::new();
        for (i, icon) in icons.iter().enumerate() {
            let mut node = d2_rs_core::Node::new(
                format!("asset{i}"),
                d2_rs_core::Size {
                    width: 64.,
                    height: 64.,
                },
            );
            node.shape = ShapeKind::Image;
            let id = graph.add_node(node);
            styles.insert(
                id,
                SourceNodeStyle {
                    icon: Some((*icon).to_owned()),
                    ..SourceNodeStyle::default()
                },
            );
        }
        (graph, styles)
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn malformed_pixels_reach_caller_with_actual_source_asset_identity() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/jpeg-caller-errors-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 16);
        for row in rows {
            let icon = format!("bad.{}", row["kind"].as_str().unwrap());
            let (graph, styles) = graph(&[&icon]);
            let mut options = RenderOptions::default();
            options.asset_data.insert(
                icon,
                row["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect(),
            );
            let cache = validate_imported_svg_assets(&graph, &styles, &options).unwrap();
            let error = prepare_jpeg_assets(&graph, &styles, &options, &cache)
                .err()
                .expect("source malformed pixels rejected");
            assert_eq!(error.to_string(), row["error"].as_str().unwrap());
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_aliases_and_repeated_uses_share_one_prepared_resource() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/raster-validation-v090.json"
        ))
        .unwrap();
        let data: Vec<u8> = rows[0]["input"]["Data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        let (graph, styles) = graph(&["one.jpg", "two.jpg", "one.jpg"]);
        let mut options = RenderOptions::default();
        options.asset_data.insert("one.jpg".into(), data.clone());
        options.asset_data.insert("two.jpg".into(), data);
        let unused_png = resvg::tiny_skia::Pixmap::new(1, 1)
            .unwrap()
            .encode_png()
            .unwrap();
        options
            .asset_data
            .insert("another-board.png".into(), unused_png.clone());
        let cache = validate_imported_svg_assets(&graph, &styles, &options).unwrap();
        let prepared = prepare_jpeg_assets(&graph, &styles, &options, &cache).unwrap();
        assert_eq!(prepared.by_asset.len(), 1);
        assert_eq!(prepared.by_source.len(), 2);
        assert_eq!(prepared.by_source["one.jpg"], prepared.by_source["two.jpg"]);
        let mut handoffs = png_image::HandoffScope::default();
        let normalized = normalize_raster_assets(options, &prepared, &mut handoffs);
        assert_eq!(normalized.asset_data["another-board.png"], unused_png);
    }
}

#[cfg(test)]
mod png_caller_source_tests {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_png_pixel_failures_propagate_for_inline_and_resolved_assets() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/png-caller-errors-v090.json"
        ))
        .unwrap();
        for row in rows {
            for inline in [false, true] {
                let data: Vec<u8> = row["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect();
                let icon = if inline {
                    row["icon"].as_str().unwrap()
                } else {
                    "pixel.png"
                };
                let mut graph = Graph::default();
                let mut node = d2_rs_core::Node::new(
                    "x",
                    d2_rs_core::Size {
                        width: 64.0,
                        height: 64.0,
                    },
                );
                node.shape = ShapeKind::Image;
                let id = graph.add_node(node);
                let styles = BTreeMap::from([(
                    id,
                    SourceNodeStyle {
                        icon: Some(icon.to_owned()),
                        ..Default::default()
                    },
                )]);
                let mut options = RenderOptions::default();
                if !inline {
                    options.asset_data.insert(icon.to_owned(), data);
                }
                let cache = validate_imported_svg_assets(&graph, &styles, &options).unwrap();
                let result = prepare_jpeg_assets(&graph, &styles, &options, &cache);
                match row["error"].as_str() {
                    Some(error) => assert_eq!(
                        result
                            .err()
                            .expect("source malformed pixels rejected")
                            .to_string(),
                        error
                    ),
                    None => assert_eq!(result.unwrap().by_source.len(), 1),
                }
            }
        }
    }
}

#[cfg(test)]
mod png_compression_source_tests {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_png_stream_termination_checksum_dictionary_and_split_idat() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/png-compression-errors-v090.json"
        ))
        .unwrap();
        for row in rows {
            for inline in [false, true] {
                let data: Vec<u8> = row["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect();
                let icon = if inline {
                    row["icon"].as_str().unwrap()
                } else {
                    "pixel.png"
                };
                let mut graph = Graph::default();
                let mut node = d2_rs_core::Node::new(
                    "x",
                    d2_rs_core::Size {
                        width: 64.0,
                        height: 64.0,
                    },
                );
                node.shape = ShapeKind::Image;
                let id = graph.add_node(node);
                let styles = BTreeMap::from([(
                    id,
                    SourceNodeStyle {
                        icon: Some(icon.to_owned()),
                        ..Default::default()
                    },
                )]);
                let mut options = RenderOptions::default();
                if !inline {
                    options.asset_data.insert(icon.to_owned(), data);
                }
                let cache = validate_imported_svg_assets(&graph, &styles, &options).unwrap();
                let result = prepare_jpeg_assets(&graph, &styles, &options, &cache);
                match row["error"].as_str() {
                    Some(error) => assert_eq!(
                        result
                            .err()
                            .expect("source malformed pixels rejected")
                            .to_string(),
                        error
                    ),
                    None => assert_eq!(result.unwrap().by_source.len(), 1),
                }
            }
        }
    }
}

#[cfg(test)]
mod png_error_order_source_tests {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_png_combined_errors_preserve_rows_and_final_byte_read_order() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/png-error-order-v090.json"
        ))
        .unwrap();
        for row in rows {
            for inline in [false, true] {
                let data: Vec<u8> = row["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect();
                let icon = if inline {
                    row["icon"].as_str().unwrap()
                } else {
                    "pixel.png"
                };
                let mut graph = Graph::default();
                let mut node = d2_rs_core::Node::new(
                    "x",
                    d2_rs_core::Size {
                        width: 64.0,
                        height: 64.0,
                    },
                );
                node.shape = ShapeKind::Image;
                let id = graph.add_node(node);
                let styles = BTreeMap::from([(
                    id,
                    SourceNodeStyle {
                        icon: Some(icon.to_owned()),
                        ..Default::default()
                    },
                )]);
                let mut options = RenderOptions::default();
                if !inline {
                    options.asset_data.insert(icon.to_owned(), data);
                }
                let cache = validate_imported_svg_assets(&graph, &styles, &options).unwrap();
                let result = prepare_jpeg_assets(&graph, &styles, &options, &cache);
                match row["error"].as_str() {
                    Some(error) => assert_eq!(
                        result
                            .err()
                            .expect("source malformed pixels rejected")
                            .to_string(),
                        error
                    ),
                    None => assert_eq!(result.unwrap().by_source.len(), 1),
                }
            }
        }
    }
}

#[cfg(test)]
mod gif_caller_source_tests {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_gif_errors_reach_both_inline_and_resolved_callers() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/gif-pixels-v090.json"
        ))
        .unwrap();
        for row in rows {
            for inline in [false, true] {
                let icon = if inline {
                    row["icon"].as_str().unwrap()
                } else {
                    "asset.gif"
                };
                let mut graph = Graph::default();
                let mut node = d2_rs_core::Node::new(
                    "x",
                    d2_rs_core::Size {
                        width: 64.,
                        height: 64.,
                    },
                );
                node.shape = ShapeKind::Image;
                let id = graph.add_node(node);
                let styles = BTreeMap::from([(
                    id,
                    SourceNodeStyle {
                        icon: Some(icon.into()),
                        ..Default::default()
                    },
                )]);
                let mut options = RenderOptions::default();
                if !inline {
                    options.asset_data.insert(
                        icon.into(),
                        row["data"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_u64().unwrap() as u8)
                            .collect(),
                    );
                }
                let cache = validate_imported_svg_assets(&graph, &styles, &options).unwrap();
                let result = prepare_jpeg_assets(&graph, &styles, &options, &cache);
                if let Some(error) = row["error"].as_str() {
                    assert!(
                        result
                            .err()
                            .unwrap()
                            .to_string()
                            .ends_with(&format!("has malformed GIF pixel data: {error}")),
                        "{}",
                        row["name"]
                    );
                } else {
                    let prepared = result.unwrap();
                    assert_eq!(prepared.by_source.len(), 1, "{}", row["name"]);
                    let normalized = normalize_raster_assets(
                        options,
                        &prepared,
                        &mut png_image::HandoffScope::default(),
                    );
                    assert!(
                        normalized.asset_data[icon].starts_with(b"\x89PNG\r\n\x1a\n"),
                        "{}",
                        row["name"]
                    );
                }
            }
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod legend_animation_source_contract {
    use super::*;
    #[test]
    fn original_go_reverse_and_forward_dash_samples_match_exact_bits() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-render"),
            "tests/fixtures/legend-connections-source-v090.json",
        ))
        .unwrap();
        let mut samples = 0;
        for row in rows.as_array().unwrap() {
            let c = &row["connection"];
            for node in row["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|n| n["dash_samples"].as_array().is_some_and(|a| !a.is_empty()))
            {
                let track = &node["animations"][0];
                let reverse = track["Keyframes"][0]["Value"]["Number"].as_f64().unwrap() != 0.0;
                let stroke = &node["primitive"]["Stroke"];
                let target = if reverse {
                    track["Keyframes"][0]["Value"]["Number"].as_f64().unwrap()
                } else {
                    track["Keyframes"][1]["Value"]["Number"].as_f64().unwrap()
                };
                let gap = stroke["Dashes"][1].as_f64().unwrap();
                let svg = format!(
                    "<svg><path stroke-width=\"{}\" stroke-dasharray=\"{},{}\" stroke-dashoffset=\"{target}\" animation=\"dashdraw {}s linear infinite{}\"/></svg>",
                    c["strokeWidth"],
                    stroke["Dashes"][0],
                    stroke["Dashes"][1],
                    gap * 0.5,
                    if reverse { " reverse" } else { "" }
                );
                for retained_id in [false, true] {
                    let input = if retained_id {
                        svg.replace("<path ", "<path id=\"retained\" ")
                    } else {
                        svg.clone()
                    };
                    for sample in node["dash_samples"].as_array().unwrap() {
                        let output = set_animated_dash_offsets(
                            &input,
                            Some(sample["time_ns"].as_i64().unwrap()),
                        );
                        let parameters = collect_exact_dash_parameters(&output);
                        let actual = parameters.values().next().unwrap().2;
                        assert_eq!(
                            actual.to_bits(),
                            sample["value"].as_f64().unwrap().to_bits(),
                            "{} {} {} retained={retained_id}",
                            row["name"],
                            node["id"],
                            sample["time_ns"]
                        );
                        samples += 1;
                    }
                }
            }
        }
        assert!(samples > 1000);
        let mut graph = Graph::default();
        graph.legend = Some(d2_rs_core::Legend {
            source_graph: None,
            label: String::new(),
            shapes: Vec::new(),
            connections: vec![d2_rs_core::LegendConnection {
                animated: true,
                ..Default::default()
            }],
        });
        assert!(has_animated_content(&graph, &BTreeMap::new()));
    }
}

#[cfg(test)]
mod integrated_legend_shadow_contract {
    use super::*;
    #[test]
    fn generated_legend_shadow_stays_on_native_backend() {
        use d2_rs_core::{LayoutOptions, Size, ast::Shared};
        use d2_rs_language::{compiler, parse};
        let parsed = parse(
            "",
            b"vars:{d2-legend:{a:{label:A;style.shadow:true}}}\nx",
            None,
        );
        assert!(parsed.error.is_none());
        let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
        let (graph, labels, styles) =
            compiler::lower_graph_with_measurer_and_labels(&compiler, root, &|_: &str| Size {
                width: 8.0,
                height: 20.0,
            })
            .unwrap();
        let layout = d2_rs_tala::Engine
            .layout(&graph, &LayoutOptions::default())
            .unwrap();
        let mut render_options = RenderOptions::default();
        render_options.force_appendix = true;
        let (svg, paths) = d2_rs_render::render_svg_with_raster_sources_checked(
            &graph,
            &layout,
            &labels,
            &styles,
            render_options,
            false,
        )
        .unwrap();
        let svg = configure_d2_markers_for_native_raster(&strip_embedded_font_faces(
            std::str::from_utf8(&svg).unwrap(),
        ));
        let (svg, sources) = svg_precision::tag_embedded_svg_images(&svg);
        assert!(sources.is_native_legend_shadow("d2-rs-native-legend-shadow-card"));
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let mut identities = BTreeSet::new();
        for id in doc
            .descendants()
            .filter_map(|n| n.attribute("id"))
            .filter(|id| id.starts_with("d2-rs-native-legend-"))
        {
            assert!(
                identities.insert(id),
                "Duplicate private legend identity: {id}"
            );
        }
        let carriers = doc
            .descendants()
            .filter(|n| {
                n.is_element()
                    && (n.attribute("id").is_some_and(|id| id.contains("legend"))
                        || n.has_tag_name("feDropShadow"))
            })
            .map(|n| {
                format!(
                    "{} attrs={:?} parents={:?}",
                    n.tag_name().name(),
                    n.attributes()
                        .map(|a| (a.name(), a.value()))
                        .collect::<Vec<_>>(),
                    n.ancestors()
                        .filter(|p| p.is_element())
                        .map(|p| (p.tag_name().name(), p.attribute("id")))
                        .collect::<Vec<_>>()
                )
            })
            .collect::<Vec<_>>();
        assert!(
            sources.is_native_legend_shadow("d2-rs-native-legend-shape-shadow-0"),
            "Generated carriers: {carriers:?}"
        );
        let mut options = resvg::usvg::Options {
            font_family: "Source Sans Pro".into(),
            ..Default::default()
        };
        load_bundled_fonts_for_primary(&mut options, false, false);
        let tree = resvg::usvg::Tree::from_str(&svg, &options).unwrap();
        let mut groups = Vec::new();
        fn visit(group: &resvg::usvg::Group, groups: &mut Vec<String>) {
            if !group.filters().is_empty() {
                groups.push(format!(
                    "{} opacity={} isolate={} filters={} bounds={:?} transform={:?}",
                    group.id(),
                    group.opacity().get(),
                    group.isolate(),
                    group.filters().len(),
                    group.abs_stroke_bounding_box(),
                    group.abs_transform()
                ));
            }
            for child in group.children() {
                if let resvg::usvg::Node::Group(g) = child {
                    visit(g, groups);
                }
            }
        }
        visit(tree.root(), &mut groups);
        assert!(
            scanline::render_simple_with_sources(&tree, 2, &paths, &sources).is_some(),
            "Generated filter groups: {groups:?}"
        );
    }
}
