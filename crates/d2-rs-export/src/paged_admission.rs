// SPDX-License-Identifier: MPL-2.0
//! d2cli/paged.go pagedFrameOptions and raster.go scene admission. Work is
//! planned from typed scene geometry before the existing pixel renderer runs.
use super::*;
use d2_rs_raster::FrameOptions;
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
    Ok(Arc::new(
        scene::BundledFallbackResolver::new(
            None,
            scene::BundledFallbackLimits {
                max_requested_runes: 100_000,
                max_bundled_bytes: 8 << 20,
                max_resolved_bytes: 136 << 20,
            },
        )
        .map_err(ExportError::Render)?,
    ))
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

pub(super) fn preflight(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
    scale: u32,
    budget: &mut PagedFrameBudget,
) -> Result<(), ExportError> {
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
    d2_rs_raster::preflight(&document, &frame)
        .map_err(|error| ExportError::Render(error.to_string()))
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

/// encodePNGBands' admission: RenderBands prepares the complete scene with
/// 256-row bands and enforces every frame ceiling before the first row.
pub(super) fn png_preflight(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
) -> Result<(), ExportError> {
    let frame = png_frame_options();
    let resources = Resources::new()?;
    let document = build_document(
        graph, result, labels, styles, options, sketch, 1, &frame, &resources, false,
    )?;
    d2_rs_raster::preflight_bands(&document, &frame, 256)
        .map_err(|error| ExportError::Render(error.to_string()))
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
    // label state. Match render_svg_with_raster_sources_checked's default at
    // this bridge; source-compiled diagrams already carry their positions.
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
