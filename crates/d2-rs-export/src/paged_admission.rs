// SPDX-License-Identifier: MPL-2.0
//! d2cli's raster exports from the typed scene: raster_png_output.go (PNG),
//! raster.go (GIF) and paged.go (PDF/PPTX frames and their links). Each
//! board's scene is built once, then admitted and rendered by d2-rs-raster.
use super::*;
use d2_rs_raster::FrameOptions;
use d2_rs_render::go_float::go_fmsub;
use d2_rs_scenebuild as scene;
use std::sync::Arc;

/// Resolve only the caller's acquired bytes and embedded data. Admission must
/// neither fetch an asset again nor turn an unavailable image into local I/O.
#[derive(Debug)]
struct AcquiredImages {
    data: BTreeMap<String, Vec<u8>>,
    resolver: Arc<scene::RasterImageResolver>,
}
impl scene::ImageResolver for AcquiredImages {
    fn resolve(
        &self,
        source: &str,
    ) -> Result<Option<scene::ImageResource>, scene::ImageResolveError> {
        if source.starts_with("data:") {
            return scene::ImageResolver::resolve(self.resolver.as_ref(), source);
        }
        let Some(bytes) = self.data.get(source) else {
            return Err(scene::ImageResolveError {
                unavailable: true,
                message: "image unavailable".into(),
            });
        };
        self.resolver
            .resolve_acquired(source, bytes)
            .map(|asset| Some(asset.resource))
    }
}

/// One operation retains resolver accounting and immutable resource memoization
/// across boards, matching paged.go's pagedRenderer.assets and .fonts.
pub(super) struct Resources {
    pub(super) images: Arc<scene::RasterImageResolver>,
    pub(super) fonts: Arc<scene::BundledFallbackResolver>,
}

impl Resources {
    pub(super) fn new() -> Result<Self, ExportError> {
        Self::with_fonts(None)
    }

    fn with_fonts(fonts: Option<Arc<scene::BundledFallbackResolver>>) -> Result<Self, ExportError> {
        let images = scene::RasterImageResolver::new(
            std::path::Path::new("."),
            scene::ImageAssetLimits {
                max_fetched_bytes: 32 << 20,
                max_encoded_bytes: 32 << 20,
                max_decompressed_bytes: 32 << 20,
                max_svg_bytes: 64 << 10,
                max_decoded_width: 32768,
                max_decoded_height: 32768,
                max_decoded_pixels: 64 << 20,
                max_assets: 4032,
                max_cumulative_encoded_bytes: 448 << 20,
                max_cumulative_decoded_bytes: 512 << 20,
            },
            Arc::new(scene::UnrestrictedImageFiles::new()),
        )
        .map_err(|error| ExportError::Render(error.to_string()))?;
        let fonts = match fonts {
            Some(fonts) => fonts,
            None => new_font_resolver()?,
        };
        Ok(Self {
            images: Arc::new(images),
            fonts,
        })
    }
}

fn new_font_resolver() -> Result<Arc<scene::BundledFallbackResolver>, ExportError> {
    // d2cli.newFontFallbackOptions chains bundled fallbacks to the host
    // resolver for every raster session, including paged PDF/PPTX exports.
    // Keep this identical to the already-qualified GIF scene path.
    Ok(GifFontResources::new()?.fonts)
}

fn frame_options(scale: u32, budget: &PagedFrameBudget) -> FrameOptions {
    let count = i64::try_from(budget.total_boards).unwrap_or(i64::MAX);
    let divide = |value: i64| (value / count).max(1);
    FrameOptions {
        scale: f64::from(scale),
        max_width: 32768,
        max_height: 32768,
        max_pixels: budget.remaining_pixels.min(64 << 20) as i64,
        max_nodes: divide(1_000_000),
        max_depth: 1024,
        max_path_commands: divide(10_000_000),
        max_text_runes_per_run: 100_000,
        max_font_faces_per_text: 64,
        max_text_coverage_checks: divide(10_000_000),
        max_text_shaping_runs: divide(100_000),
        max_animation_tracks: divide(1_000_000),
        max_animation_keyframes: divide(10_000_000),
        max_assets: 4096,
        max_asset_bytes: 512 << 20,
        max_decoded_asset_bytes: 512 << 20,
        max_import_depth: 1024,
        max_offscreen_bytes: 512 << 20,
        max_even_odd_clip_work: divide(250_000_000),
        max_scanline_work: divide(4_000_000_000),
        ..Default::default()
    }
}

/// One paged board as d2cli/paged.go pagedRenderer.render draws it: the
/// board's scene is built once, rendered with the document-divided frame
/// ceilings, and its links are mapped into that frame (mapPagedLinks).
/// Returns the straight-alpha RGBA frame and the board's links.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_frame(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
    scale: u32,
    budget: &mut PagedFrameBudget,
) -> Result<PagedFrame, ExportError> {
    let frame = frame_options(scale, budget);
    let count = i64::try_from(budget.total_boards).unwrap_or(i64::MAX);
    if budget.resources.is_none() {
        budget.resources = Some(Resources::new()?);
    }
    let resources = budget
        .resources
        .as_ref()
        .expect("initialized paged resources");
    let document = build_document(
        graph, result, labels, styles, options, sketch, count, &frame, resources, true,
    )?;
    let image = d2_rs_raster::render(&document, &frame)
        .map_err(|error| ExportError::Render(error.to_string()))?;
    let width = image.width();
    let height = image.height();
    let links = map_paged_links(&document, width, height, frame.scale).map_err(links_error)?;
    let rgba = if image.stride == width * 4 {
        let mut pix = image.pix;
        pix.truncate(height * width * 4);
        pix
    } else {
        let mut rgba = Vec::with_capacity(width * height * 4);
        for row in 0..height {
            let start = row * image.stride;
            rgba.extend_from_slice(&image.pix[start..start + width * 4]);
        }
        rgba
    };
    Ok(PagedFrame {
        width: width as u32,
        height: height as u32,
        rgba,
        links,
    })
}

/// A rendered paged board: its straight-alpha frame and mapped links.
pub(super) struct PagedFrame {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) rgba: Vec<u8>,
    pub(super) links: Vec<DocumentLinkMetadata>,
}

/// d2cli/raster_png_output.go pngFrameOptions: rasterFrameOptions(2, 0)
/// with the full dimension allowance instead of the complete-frame cap.
fn png_frame_options() -> FrameOptions {
    FrameOptions {
        scale: 2.0,
        time: 0,
        background: Some(d2_rs_raster::Nrgba::new(255, 255, 255, 255)),
        max_width: 32768,
        max_height: 32768,
        max_pixels: 32768 * 32768,
        max_nodes: 1_000_000,
        max_depth: 1024,
        max_path_commands: 10_000_000,
        max_text_runes_per_run: 100_000,
        max_font_faces_per_text: 64,
        max_text_coverage_checks: 10_000_000,
        max_text_shaping_runs: 100_000,
        max_animation_tracks: 1_000_000,
        max_animation_keyframes: 10_000_000,
        max_assets: 4096,
        max_asset_bytes: 512 << 20,
        max_decoded_asset_bytes: 512 << 20,
        max_import_depth: 1024,
        max_offscreen_bytes: 512 << 20,
        max_even_odd_clip_work: 250_000_000,
        max_scanline_work: 4_000_000_000,
        ..Default::default()
    }
}

/// d2cli/raster.go gifFrameOptions: rasterFrameOptions(1, t) with the GIF
/// ceilings, aggregate work divided across the whole frame schedule.
fn gif_frame_options(total_frames: usize) -> FrameOptions {
    let divisor = i64::try_from(total_frames.max(1)).unwrap_or(i64::MAX);
    let divide = |value: i64| (value / divisor).max(1);
    FrameOptions {
        scale: 1.0,
        time: 0,
        background: Some(d2_rs_raster::Nrgba::new(255, 255, 255, 255)),
        max_width: 32768,
        max_height: 32768,
        max_pixels: 2 * 1024 * 1024,
        max_nodes: divide(1_000_000),
        max_depth: 1024,
        max_path_commands: divide(10_000_000),
        max_text_runes_per_run: 100_000,
        max_font_faces_per_text: 64,
        max_text_coverage_checks: divide(10_000_000),
        max_text_shaping_runs: divide(100_000),
        max_animation_tracks: divide(1_000_000),
        max_animation_keyframes: divide(10_000_000),
        max_assets: 1024,
        max_asset_bytes: 128 << 20,
        max_decoded_asset_bytes: 128 << 20,
        max_import_depth: 1024,
        max_offscreen_bytes: 128 << 20,
        max_even_odd_clip_work: divide(768 << 20),
        max_scanline_work: divide(8_000_000_000),
        ..Default::default()
    }
}

/// One GIF board as d2cli/raster.go renderGIFWithSession plans it: the scene
/// document is built once with the board-divided budgets, its logical canvas
/// admitted (gifDocumentFrameBounds), and every sampled frame rendered or
/// preflighted by d2raster with the frame-divided ceilings.
pub struct GifBoard {
    document: d2_rs_raster::SceneDocument,
    frame: FrameOptions,
}

/// Font fallback state shared by the boards of one GIF export.
pub struct GifFontResources {
    fonts: Arc<scene::BundledFallbackResolver>,
}

impl GifFontResources {
    /// d2cli.newFontFallbackOptions also supplies the isometric CLI. Keep the
    /// bundled-first chain and its cumulative system search budget shared.
    pub fn resolver(&self) -> Arc<dyn scene::FallbackResolver> {
        self.fonts.clone()
    }

    pub fn new() -> Result<Self, ExportError> {
        #[cfg(any(
            target_os = "macos",
            target_os = "linux",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        let next: Option<Arc<dyn scene::FallbackResolver>> = Some(Arc::new(
            scene::SystemFallbackResolver::new(scene::SystemFallbackLimits {
                max_directory_entries: 20_000,
                max_files: 4096,
                max_faces: 8192,
                max_requested_runes: 100_000,
                max_coverage_checks: 10_000_000,
                max_file_bytes: 32 << 20,
                max_scanned_bytes: 256 << 20,
                max_resolved_bytes: 128 << 20,
            })
            .map_err(ExportError::Render)?,
        ));
        #[cfg(not(any(
            target_os = "macos",
            target_os = "linux",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        )))]
        let next = None;
        Ok(Self {
            fonts: Arc::new(
                scene::BundledFallbackResolver::new(
                    next,
                    scene::BundledFallbackLimits {
                        max_requested_runes: 100_000,
                        max_bundled_bytes: 8 << 20,
                        max_resolved_bytes: 136 << 20,
                    },
                )
                .map_err(ExportError::Render)?,
            ),
        })
    }
}

impl GifBoard {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan(
        graph: &Graph,
        result: &LayoutResult,
        labels: &BTreeMap<NodeId, String>,
        styles: &BTreeMap<NodeId, SourceNodeStyle>,
        options: &RenderOptions,
        sketch: bool,
        total_boards: usize,
        total_frames: usize,
    ) -> Result<Self, ExportError> {
        let fonts = GifFontResources::new()?;
        Self::plan_with_fonts(
            graph,
            result,
            labels,
            styles,
            options,
            sketch,
            total_boards,
            total_frames,
            Some(&fonts),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan_with_fonts(
        graph: &Graph,
        result: &LayoutResult,
        labels: &BTreeMap<NodeId, String>,
        styles: &BTreeMap<NodeId, SourceNodeStyle>,
        options: &RenderOptions,
        sketch: bool,
        total_boards: usize,
        total_frames: usize,
        fonts: Option<&GifFontResources>,
    ) -> Result<Self, ExportError> {
        let frame = gif_frame_options(total_frames);
        let resources = Resources::with_fonts(fonts.map(|state| Arc::clone(&state.fonts)))?;
        let boards = i64::try_from(total_boards.max(1)).unwrap_or(i64::MAX);
        let document = build_document(
            graph, result, labels, styles, options, sketch, boards, &frame, &resources, false,
        )?;
        validate_gif_frame_dimensions(document.logical_width, document.logical_height)?;
        Ok(Self { document, frame })
    }

    /// The planned frame bounds: the ceiling of the logical canvas.
    pub fn size(&self) -> (u32, u32) {
        (
            self.document.logical_width.ceil() as u32,
            self.document.logical_height.ceil() as u32,
        )
    }

    /// d2raster's per-frame preflight of the first sample, whose limit
    /// failures raster.go reports as "GIF frame 0: ...".
    pub(super) fn preflight(&self) -> Result<(), ExportError> {
        d2_rs_raster::preflight(&self.document, &self.frame)
            .map_err(|error| ExportError::Render(format!("GIF frame 0: {error}")))
    }

    /// Renders one sample as opaque RGBA rows (straight and premultiplied
    /// alpha coincide over the white background).
    pub(super) fn render(
        &mut self,
        frame_index: usize,
        time_ns: i64,
    ) -> Result<(u32, u32, Vec<u8>), ExportError> {
        self.frame.time = time_ns;
        let image = d2_rs_raster::render(&self.document, &self.frame)
            .map_err(|error| ExportError::Render(format!("GIF frame {frame_index}: {error}")))?;
        let width = image.width();
        let height = image.rect.dy() as usize;
        let mut rgba = Vec::with_capacity(width * height * 4);
        for row in 0..height {
            let start = row * image.stride;
            rgba.extend_from_slice(&image.pix[start..start + width * 4]);
        }
        Ok((width as u32, height as u32, rgba))
    }
}

/// d2cli loadFonts registers --font-* faces as the "custom" and "customMono"
/// families (d2fonts.AddFontFamily) and d2exporter.Export names them on the
/// diagram, a mono theme still selecting SourceCodePro; d2scenebuild then
/// looks the faces up by family. The registration is process-wide in Go too;
/// a repeated set of faces is registered once.
fn apply_custom_fonts(
    diagram: &mut d2_rs_target::Diagram,
    fonts: &d2_rs_text::d2fonts::CustomFonts,
) -> Result<(), ExportError> {
    use d2_rs_text::d2fonts::{FontStyle, add_font_family};
    static REGISTERED: std::sync::Mutex<Option<d2_rs_text::d2fonts::CustomFonts>> =
        std::sync::Mutex::new(None);
    if fonts.regular.is_empty() && fonts.mono.is_empty() {
        return Ok(());
    }
    {
        let mut registered = REGISTERED.lock().unwrap_or_else(|error| error.into_inner());
        if registered.as_ref() != Some(fonts) {
            for (name, faces) in [("custom", &fonts.regular), ("customMono", &fonts.mono)] {
                if faces.is_empty() {
                    continue;
                }
                let face = |style| faces.get(&style).map(Vec::as_slice);
                add_font_family(
                    name,
                    face(FontStyle::Regular),
                    face(FontStyle::Italic),
                    face(FontStyle::Bold),
                    face(FontStyle::Semibold),
                )
                .map_err(ExportError::Render)?;
            }
            *registered = Some(fonts.clone());
        }
    }
    // from_graph names SourceCodePro only for a mono theme, which overrides
    // the custom family in d2exporter.Export.
    if !fonts.regular.is_empty() && diagram.font_family.as_deref() != Some("SourceCodePro") {
        diagram.font_family = Some("custom".into());
    }
    if !fonts.mono.is_empty() {
        diagram.mono_font_family = Some("customMono".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_document(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
    count: i64,
    frame: &FrameOptions,
    resources: &Resources,
    transparent_root: bool,
) -> Result<d2_rs_raster::SceneDocument, ExportError> {
    let divide = |value: i64| (value / count).max(1);
    let json = d2_rs_render::target_diagram_json(graph, result, labels, styles, options, sketch)
        .map_err(ExportError::Render)?;
    let mut diagram: d2_rs_target::Diagram =
        serde_json::from_slice(&json).map_err(|error| ExportError::Render(error.to_string()))?;
    // Graph API callers can attach icons after layout, without a positioned
    // label state. Default them to inside-middle-center at this bridge, as the
    // earlier SVG raster bridge did; source-compiled diagrams already carry
    // their positions.
    for shape in diagram.shapes.iter_mut().flatten() {
        if shape.icon.is_some() && shape.icon_position.is_empty() {
            shape.icon_position = d2_rs_core::LabelPosition::InsideMiddleCenter
                .d2_name()
                .into();
        }
    }
    if transparent_root {
        diagram.root.fill = "transparent".into();
    }
    apply_custom_fonts(&mut diagram, &options.custom_fonts)?;
    let overrides = options
        .theme_overrides
        .iter()
        .map(|(key, value)| (key.to_ascii_lowercase(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let theme_overrides = serde_json::from_value(
        serde_json::to_value(overrides).map_err(|error| ExportError::Render(error.to_string()))?,
    )
    .map_err(|error| ExportError::Render(error.to_string()))?;
    scene::build(
        &diagram,
        &scene::Options {
            pad: Some(options.pad as i64),
            scale: Some(options.scale.unwrap_or(1.0)),
            center: Some(options.center),
            theme_id: options.theme_id,
            theme_overrides: Some(theme_overrides),
            max_nodes: frame.max_nodes,
            max_path_commands: frame.max_path_commands,
            sketch,
            sketch_budget: scene::SketchBudget {
                max_operation_sets: 1_000_000,
                max_operations: 10_000_000,
                max_path_commands: 10_000_000,
            },
            link_budget: scene::LinkBudget {
                max_regions: divide(65536).min(4096),
                max_string_bytes: divide(64 << 20).min(1 << 20),
            },
            appendix: true,
            assets: Some(scene::AssetOptions {
                resolver_impl: Some(Arc::new(AcquiredImages {
                    data: options.asset_data.clone(),
                    resolver: Arc::clone(&resources.images),
                })),
                svg_import_limits: scene::SvgImportLimits {
                    max_bytes: 65536,
                    max_depth: 32,
                    max_elements: 256,
                    max_attributes: 512,
                    max_attribute_bytes: 65536,
                    max_path_commands: 4096,
                    max_transform_functions: 128,
                    max_use_depth: 8,
                    max_resources: 64,
                },
                svg_import_budget: scene::SvgImportBudget {
                    max_source_bytes: (1 << 20) / count,
                    max_elements: 4096 / count,
                    max_attributes: 8192 / count,
                    max_attribute_bytes: (1 << 20) / count,
                    max_path_commands: 65536 / count,
                    max_transform_functions: 2048 / count,
                    max_declared_resources: 1024 / count,
                    max_expanded_use_instances: 1024 / count,
                },
                ..Default::default()
            }),
            fonts: Some(scene::FontFallbackOptions {
                resolver_impl: Some(resources.fonts.clone()),
                max_assets: 64,
                max_bytes: 64 << 20,
                max_runes_per_text: 100_000,
                max_total_runes: divide(10_000_000),
                max_coverage_checks: divide(50_000_000),
                max_font_faces_per_text: 64,
                max_shaping_runs: divide(100_000),
                max_shaped_glyphs: divide(10_000_000),
                ..Default::default()
            }),
        },
    )
    .map_err(|error| ExportError::Render(error.to_string()))
}

/// d2cli/raster_png_output.go renderPNGToWriter and encodePNGBands: the
/// scene is built once with buildScene's undivided budgets and rendered in
/// 256-row bands with pngFrameOptions. RenderBands admits the complete frame
/// before the first band; each band streams into the encoder, which writes
/// RGB rows because the white background makes every pixel opaque.
pub(super) fn render_png(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
    scale: u32,
) -> Result<Vec<u8>, ExportError> {
    let mut frame = png_frame_options();
    frame.scale = f64::from(scale);
    let resources = Resources::new()?;
    let document = build_document(
        graph, result, labels, styles, options, sketch, 1, &frame, &resources, false,
    )?;
    // encodePNGBands: the stream height is ceil(LogicalHeight * Scale).
    let height = (document.logical_height * frame.scale).ceil() as u32;
    // Like d2cli's rasterPNGBandEncoder, encode each band once it is drawn.
    // A second thread filters and compresses a band while the next renders;
    // the rendezvous queue keeps one band in flight, and a frame that fits in
    // one band is encoded in place.
    std::thread::scope(|scope| {
        let (bands, queue) = std::sync::mpsc::sync_channel::<(usize, Vec<u8>)>(0);
        let (reuse, recycled) = std::sync::mpsc::channel::<Vec<u8>>();
        let encoder = scope.spawn(move || {
            let mut stream: Option<go_png::RowStream> = None;
            for (width, band) in queue {
                stream
                    .get_or_insert_with(|| go_png::RowStream::new(width as u32, height, 3))
                    .push_rows(&band, 4);
                let _ = reuse.send(band);
            }
            stream.map(go_png::RowStream::finish)
        });
        let mut whole: Option<Vec<u8>> = None;
        let mut band_in_flight = false;
        let rendered = d2_rs_raster::render_bands(&document, &frame, 256, &mut |band| {
            let width = band.width();
            let rows = band.rect.dy() as usize;
            let row_bytes = width * 4;
            if whole.is_none() && !band_in_flight && rows as u32 == height {
                let mut stream = go_png::RowStream::new(width as u32, height, 3);
                if band.stride == row_bytes {
                    stream.push_rows(&band.pix[..rows * row_bytes], 4);
                } else {
                    for row in 0..rows {
                        let start = row * band.stride;
                        stream.push_rows(&band.pix[start..start + row_bytes], 4);
                    }
                }
                whole = Some(stream.finish());
                return Ok(());
            }
            // Reuse the preceding band's storage once the encoder is done
            // with it, so at most two bands are ever retained.
            let mut buffer = if band_in_flight {
                recycled.recv().unwrap_or_default()
            } else {
                band_in_flight = true;
                Vec::new()
            };
            buffer.clear();
            for row in 0..rows {
                let start = row * band.stride;
                buffer.extend_from_slice(&band.pix[start..start + row_bytes]);
            }
            // The encoder runs until the queue closes; a send cannot fail.
            let _ = bands.send((width, buffer));
            Ok(())
        });
        drop(bands);
        drop(recycled);
        let encoded = encoder.join().expect("PNG band encoder thread");
        rendered.map_err(|error| ExportError::Render(error.to_string()))?;
        whole
            .or(encoded)
            .ok_or_else(|| ExportError::Render("d2raster: PNG renderer produced no rows".into()))
    })
}

/// d2cli/paged.go mapPagedLinks (lines 270-342): the viewbox-to-frame
/// mapping d2raster applies, so every link lands in the frame's source
/// pixels. The packagers take logical coordinates, so each mapped box is
/// returned divided by `scale` (png.SCALE = 2 divides exactly): PDF's
/// pdfLinkShapes divides by png.SCALE and PPTX multiplies it back before
/// rounding outward. Tooltip-only regions stay, with an empty `href`, so
/// link indexes in packaging errors match Go's.
fn map_paged_links(
    document: &d2_rs_raster::SceneDocument,
    frame_width: usize,
    frame_height: usize,
    scale: f64,
) -> Result<Vec<DocumentLinkMetadata>, String> {
    let Some(links) = document.links.as_deref().filter(|links| !links.is_empty()) else {
        return Ok(Vec::new());
    };
    if frame_width == 0 || frame_height == 0 {
        return Err("renderer returned an invalid source-pixel frame".into());
    }
    let width = document.logical_width * scale;
    let height = document.logical_height * scale;
    let view = &document.view_box;
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || view.width <= 0.0
        || view.height <= 0.0
    {
        return Err("document has invalid viewport dimensions".into());
    }
    let (mut scale_x, mut scale_y) = (width / view.width, height / view.height);
    let (mut offset_x, mut offset_y) = (0.0, 0.0);
    if document.viewport_fit == d2_rs_scene::ViewportFit::MEET {
        let (frame_width, frame_height) = (frame_width as f64, frame_height as f64);
        let uniform = d2_rs_scene::go_min(frame_width / view.width, frame_height / view.height);
        (scale_x, scale_y) = (uniform, uniform);
        if document.viewport_align == d2_rs_scene::ViewportAlign::X_MID_Y_MID {
            // ARM64 paged.go:299-300: FMSUBD.
            offset_x = go_fmsub(view.width, uniform, frame_width) / 2.0;
            offset_y = go_fmsub(view.height, uniform, frame_height) / 2.0;
        }
    }
    // ARM64 paged.go:305-306: FMSUBD. The inlined Matrix.Point fuses its
    // first product, as d2_rs_scene::Matrix::point does.
    let mapping = d2_rs_scene::Matrix {
        a: scale_x,
        b: 0.0,
        c: 0.0,
        d: scale_y,
        e: go_fmsub(view.x, scale_x, offset_x),
        f: go_fmsub(view.y, scale_y, offset_y),
    };
    if !mapping.is_finite() {
        return Err("document link mapping is non-finite".into());
    }
    let mut mapped = Vec::with_capacity(links.len());
    for (index, link) in links.iter().enumerate() {
        let b = &link.r#box;
        let top_left = mapping.point(d2_rs_scene::Point::new(b.x, b.y));
        let bottom_right = mapping.point(d2_rs_scene::Point::new(b.x + b.width, b.y + b.height));
        let (x, y) = (top_left.x, top_left.y);
        let (width, height) = (bottom_right.x - x, bottom_right.y - y);
        if !(x.is_finite() && y.is_finite() && width.is_finite() && height.is_finite())
            || width <= 0.0
            || height <= 0.0
        {
            return Err(format!("link {index} maps to an invalid source-pixel box"));
        }
        // A board target wins over the URL, as in pdfLinkShapes and
        // addPPTXLinks; the packagers resolve and validate either.
        let board_target = !link.target.is_empty();
        let href = if board_target {
            &link.target
        } else {
            &link.url
        };
        mapped.push(DocumentLinkMetadata {
            tooltip: link.tooltip.clone(),
            board_target,
            region: DocumentLinkRegion {
                href: href.clone(),
                rect: Rect {
                    origin: Point {
                        x: x / scale,
                        y: y / scale,
                    },
                    size: d2_rs_core::Size {
                        width: width / scale,
                        height: height / scale,
                    },
                },
            },
        });
    }
    Ok(mapped)
}

/// paged.go's per-board link error, which the CLI numbers by board.
pub(super) const LINKS_ERROR_PREFIX: &str = "paged links: ";

fn links_error(error: String) -> ExportError {
    ExportError::Render(format!("{LINKS_ERROR_PREFIX}{error}"))
}

/// The links of one paged board without its pixels: the same scene and
/// admission as `render_frame`, mapped into the frame it would render.
pub(super) fn document_links(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
) -> Result<Vec<DocumentLinkMetadata>, ExportError> {
    let budget = PagedFrameBudget::default();
    let frame = frame_options(2, &budget);
    let resources = Resources::new()?;
    let document = build_document(
        graph, result, labels, styles, options, sketch, 1, &frame, &resources, true,
    )?;
    d2_rs_raster::preflight(&document, &frame)
        .map_err(|error| ExportError::Render(error.to_string()))?;
    // d2raster sizes the frame as the ceiling of the scaled logical canvas.
    let width = (document.logical_width * frame.scale).ceil() as usize;
    let height = (document.logical_height * frame.scale).ceil() as usize;
    map_paged_links(&document, width, height, frame.scale).map_err(links_error)
}
