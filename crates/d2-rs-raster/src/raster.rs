// SPDX-License-Identifier: MPL-2.0
//! d2raster/raster.go and bands.go: frame options, preflight (validation,
//! resource accounting and the prepared tree), the frame and band renderers
//! and primitive painting.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use d2_rs_scene::{
    aspect_ratio_matrix, Asset, AspectAlign, AspectFit, AspectRatio, BlendMode, Box as SceneBox, Clip, Document, DropShadow, Ellipse, FillRule, Image, LineCap, LineJoin, MaskType, Matrix, Node, Nrgba, Paint, Path, PatternPaint, Point, Primitive, Rect as SceneRect, Stroke, ViewportAlign, ViewportFit,
};

use crate::animation::animation_value_at;
use crate::effects::{plan_render_resources, plan_render_resources_region, render_effect_node, set_rasterizer_reference, supported_blend_mode, uses_effect_layer, OffscreenBudget, RenderResources};
use crate::filter::{normalize_node_filters, prepare_filters, PreparedFilter};
use crate::fma::{finite, finite_point, go_fmsub, mmul, mpoint, mvector, scale};
use crate::geometry::{
    add_fill_subpath, add_stroke_run, clean_points, ellipse_subpaths, flatten_scene_path, flatten_tolerance, make_stroke_runs, painted_stroke_pixel_bounds, rounded_rect_subpaths, shifted, stroke_join_polygon_from_units, stroke_run_pixel_bounds, subpath_pixel_bounds, unit_vector, PreparedStroke, StrokeRun, Subpath,
};
use crate::image::{draw_prepared_image, image_placement, make_inverse_affine, prepare_raster_asset, transformed_image_pixel_bounds, InverseAffine, PreparedImage, PreparedRasterAsset};
use crate::paint::{direct_pattern_pixel_offsets, draw_paint_mask, draw_rasterized_paint, pattern_tile_pixels, prepare_animated_paint_with_pattern, prepare_pattern_transform, render_pattern_tile, PaintKind, PatternTile, PreparedPaint, PreparedPattern};
use crate::pixels::{union_rect, NrgbaImage, Pt, Rect, Rgba, RgbaBuf};
use crate::scanline::{Rasterizer, WorkBudget};
use crate::text::PreparedFont;
use crate::vector_validation::finite_linear_transform_invertible;

/// The internal COLRv1 soft-light blend of prepared color-glyph trees.
pub const PREPARED_COLRV1_SOFT_LIGHT: BlendMode = BlendMode(255);

/// FrameOptions: the pixel mapping and hard resource ceilings for one
/// render. `time` is a Go `time.Duration` in nanoseconds; `background` is
/// the NRGBA-converted `color.Color` (None for a nil Background).
#[derive(Clone, Debug, Default)]
pub struct FrameOptions {
    pub scale: f64,
    pub time: i64,
    pub background: Option<Nrgba>,
    pub max_width: i64,
    pub max_height: i64,
    pub max_pixels: i64,
    pub max_nodes: i64,
    pub max_depth: i64,
    pub max_path_commands: i64,
    pub max_text_runes_per_run: i64,
    pub max_font_faces_per_text: i64,
    pub max_text_coverage_checks: i64,
    pub max_text_shaping_runs: i64,
    pub max_animation_tracks: i64,
    pub max_animation_keyframes: i64,
    pub max_assets: i64,
    pub max_asset_bytes: i64,
    pub max_decoded_asset_bytes: i64,
    pub max_import_depth: i64,
    pub max_offscreen_bytes: i64,
    pub max_even_odd_clip_work: i64,
    pub max_scanline_work: i64,
}

const DEFAULT_MAX_TEXT_RUNES_PER_RUN: i64 = 100_000;
const DEFAULT_MAX_FONT_FACES_PER_TEXT: i64 = 64;
const DEFAULT_MAX_TEXT_COVERAGE_CHECKS: i64 = 10_000_000;
const DEFAULT_MAX_TEXT_SHAPING_RUNS: i64 = 100_000;
const DEFAULT_MAX_SCANLINE_WORK: i64 = 4_000_000_000;

/// A render error carrying Go's error text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderError(pub String);

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RenderError {}

// ------------------------------------------------------------ prepared tree

#[derive(Debug)]
pub struct PreparedDocument {
    pub width: i64,
    pub height: i64,
    pub background: Option<Nrgba>,
    pub root: PreparedNode,
    pub patterns: Vec<Rc<PatternTile>>,
    pub resources: RenderResources,
}

#[derive(Debug)]
pub struct PreparedNode {
    pub opacity: f64,
    pub blend: BlendMode,
    pub isolated: bool,
    pub primitive: Option<Box<PreparedPrimitive>>,
    pub children: Vec<PreparedNode>,
    pub filters: Vec<PreparedFilter>,
    pub clip: Option<PreparedClip>,
    pub mask: Option<Box<PreparedMask>>,
    pub content_bounds: Rect,
    pub bounds: Rect,
}

impl PreparedNode {
    pub(crate) fn group(children: Vec<PreparedNode>, clip: Option<PreparedClip>) -> Self {
        Self {
            opacity: 1.0,
            blend: BlendMode::NORMAL,
            isolated: false,
            primitive: None,
            children,
            filters: Vec::new(),
            clip,
            mask: None,
            content_bounds: Rect::default(),
            bounds: Rect::default(),
        }
    }
}

#[derive(Debug, Default)]
pub struct PreparedPrimitive {
    pub reference_bounds: Rect,
    pub subpaths: Vec<Subpath>,
    pub stroke_runs: Vec<StrokeRun>,
    pub transform: Matrix,
    pub fill: Option<PreparedPaint>,
    pub fill_rule: FillRule,
    pub even_odd_subpaths: Vec<Subpath>,
    pub even_odd_edges: i64,
    pub stroke: Option<PreparedStroke>,
    pub image: Option<PreparedImage>,
    pub vector: Option<Box<PreparedNode>>,
    pub bounds: Rect,
}

#[derive(Debug, Default)]
pub struct PreparedClip {
    pub reference_bounds: Rect,
    pub subpaths: Vec<Subpath>,
    pub fill_rule: FillRule,
    pub bounds: Rect,
    pub edges: i64,
}

#[derive(Debug)]
pub struct PreparedMask {
    pub kind: MaskType,
    pub root: PreparedNode,
}

#[derive(Clone, Debug, Default)]
pub struct AnimationOverrides {
    pub fill_color: Option<Nrgba>,
    pub stroke_color: Option<Nrgba>,
    pub dash_offset: Option<f64>,
    pub drop_shadows: BTreeMap<i64, DropShadow>,
}

// ---------------------------------------------------------------- scratch

/// rasterScratch: the frame's shared rasterizer, offscreen budget, pattern
/// tiles and scanline work budget.
pub struct Scratch {
    pub region_filters: bool,
    rasterizer: Option<Rasterizer>,
    rasterizer_edges: i64,
    rasterizer_bounded: bool,
    pub offscreen: OffscreenBudget,
    pub pattern_tiles: Vec<Rc<PatternTile>>,
    pub pattern_bytes: i64,
    scanline_work: WorkBudget,
}

impl Scratch {
    pub fn reset(&mut self, bounds: Rect) -> &mut Rasterizer {
        self.reset_with_budget(bounds).0
    }

    pub fn reset_with_budget(&mut self, bounds: Rect) -> (&mut Rasterizer, &mut WorkBudget) {
        match &mut self.rasterizer {
            Some(rasterizer) => rasterizer.reset(bounds.dx(), bounds.dy()),
            None => {
                let mut rasterizer = Rasterizer::new(bounds.dx(), bounds.dy());
                if self.rasterizer_bounded {
                    rasterizer.reserve_edges(self.rasterizer_edges);
                }
                self.rasterizer = Some(rasterizer);
            }
        }
        (self.rasterizer.as_mut().unwrap(), &mut self.scanline_work)
    }

    fn release_pattern_tiles(&mut self) {
        for tile in self.pattern_tiles.drain(..) {
            *tile.image.borrow_mut() = None;
        }
        if self.pattern_bytes != 0 {
            self.offscreen.release(self.pattern_bytes);
        }
        self.pattern_bytes = 0;
    }
}

// ----------------------------------------------------------------- render

/// Render preflights document and returns a newly allocated NRGBA frame.
pub fn render(document: &Document, options: &FrameOptions) -> Result<NrgbaImage, RenderError> {
    render_frame(document, options).map_err(RenderError)
}

fn new_scratch(prepared: &PreparedDocument, options: &FrameOptions, region_filters: bool) -> Scratch {
    Scratch {
        region_filters,
        rasterizer: None,
        rasterizer_edges: prepared.resources.rasterizer_edges,
        rasterizer_bounded: true,
        offscreen: OffscreenBudget { limit: options.max_offscreen_bytes, ..Default::default() },
        pattern_tiles: Vec::new(),
        pattern_bytes: 0,
        scanline_work: WorkBudget::new(prepared.resources.scanline_work),
    }
}

fn fill_background(canvas: &mut RgbaBuf, background: Option<Nrgba>) {
    match background {
        Some(color) if color.a == 0xff => {
            for pixel in canvas.pix.chunks_exact_mut(4) {
                pixel.copy_from_slice(&[color.r, color.g, color.b, 0xff]);
            }
        }
        Some(color) if color.a != 0 => {
            // draw.Draw(..., image.NewUniform(color), draw.Src).
            let (r, g, b, a) = crate::scanline::nrgba_rgba(color);
            let pixel = [(r >> 8) as u8, (g >> 8) as u8, (b >> 8) as u8, (a >> 8) as u8];
            for chunk in canvas.pix.chunks_exact_mut(4) {
                chunk.copy_from_slice(&pixel);
            }
        }
        _ => canvas.pix.fill(0),
    }
}

fn render_frame(document: &Document, options: &FrameOptions) -> Result<NrgbaImage, String> {
    let mut prepared = prepare(document, options, 0)?;
    let mut scratch = new_scratch(&prepared, options, false);
    let rasterizer_reservation = scratch.offscreen.reserve_bytes(prepared.resources.rasterizer_bytes, "scanline rasterizer working storage")?;
    for pattern in &prepared.patterns {
        render_pattern_tile(pattern, &mut scratch)?;
    }
    let mut canvas = RgbaBuf::new(Rect::new(0, 0, prepared.width, prepared.height));
    fill_background(&mut canvas, prepared.background);
    render_node(&mut canvas.view(), &prepared.root, &mut scratch)?;
    let expected_live = rasterizer_reservation + scratch.pattern_bytes;
    if scratch.offscreen.live != expected_live {
        return Err(format!(
            "d2raster: internal offscreen reservation leak: {} bytes live, want retained rasterizer and patterns {expected_live}",
            scratch.offscreen.live
        ));
    }
    if scratch.offscreen.peak > prepared.resources.peak_offscreen_bytes {
        return Err(format!(
            "d2raster: internal offscreen peak {} exceeds preflight plan {}",
            scratch.offscreen.peak, prepared.resources.peak_offscreen_bytes
        ));
    }
    scratch.release_pattern_tiles();
    scratch.offscreen.release(rasterizer_reservation);
    let conversion = conversion_bounds(&prepared, canvas.rect);
    let opaque = prepared.background.is_some_and(|b| b.a == 0xff);
    prepared.patterns.clear();
    Ok(rgba_to_nrgba(canvas, conversion, opaque))
}

fn conversion_bounds(prepared: &PreparedDocument, canvas: Rect) -> Rect {
    match prepared.background {
        Some(b) if b.a != 0 => canvas,
        _ => prepared.root.bounds.intersect(canvas),
    }
}

/// rgbaToNRGBAInPlaceBounds: straight alpha as image.NRGBA.SetRGBA64.
fn rgba_to_nrgba(source: RgbaBuf, conversion: Rect, opaque: bool) -> NrgbaImage {
    let mut image = NrgbaImage { pix: source.pix, stride: source.stride, rect: source.rect };
    if opaque {
        return image;
    }
    let bounds = conversion.intersect(image.rect);
    for y in bounds.min.y..bounds.max.y {
        let row = ((y - image.rect.min.y) as usize) * image.stride + ((bounds.min.x - image.rect.min.x) as usize) * 4;
        for pixel in image.pix[row..row + bounds.dx() as usize * 4].chunks_exact_mut(4) {
            let alpha = pixel[3] as u32;
            if alpha != 0 && alpha != 0xff {
                pixel[0] = ((pixel[0] as u32 * 0xffff / alpha) >> 8) as u8;
                pixel[1] = ((pixel[1] as u32 * 0xffff / alpha) >> 8) as u8;
                pixel[2] = ((pixel[2] as u32 * 0xffff / alpha) >> 8) as u8;
            }
        }
    }
    image
}

/// RenderBands renders top to bottom with a canvas of at most bandHeight
/// rows; each band has absolute frame coordinates and full frame width.
pub fn render_bands(
    document: &Document,
    options: &FrameOptions,
    band_height: i64,
    consume: &mut dyn FnMut(&NrgbaImage) -> Result<(), String>,
) -> Result<(), RenderError> {
    render_bands_inner(document, options, band_height, consume).map_err(RenderError)
}

fn render_bands_inner(document: &Document, options: &FrameOptions, band_height: i64, consume: &mut dyn FnMut(&NrgbaImage) -> Result<(), String>) -> Result<(), String> {
    if band_height <= 0 {
        return Err("d2raster: band height must be positive".into());
    }
    let prepared = prepare(document, options, band_height)?;
    let mut scratch = new_scratch(&prepared, options, true);
    let reservation = scratch.offscreen.reserve_bytes(prepared.resources.rasterizer_bytes, "scanline rasterizer working storage")?;
    for pattern in &prepared.patterns {
        render_pattern_tile(pattern, &mut scratch)?;
    }
    let band_height = band_height.min(prepared.height);
    let mut top = 0;
    while top < prepared.height {
        let bottom = top + band_height.min(prepared.height - top);
        let mut canvas = RgbaBuf::new(Rect::new(0, top, prepared.width, bottom));
        fill_background(&mut canvas, prepared.background);
        render_node(&mut canvas.view(), &prepared.root, &mut scratch)?;
        let expected_live = reservation + scratch.pattern_bytes;
        if scratch.offscreen.live != expected_live {
            return Err(format!("d2raster: internal band offscreen reservation leak: {} bytes live, want {expected_live}", scratch.offscreen.live));
        }
        if scratch.offscreen.peak > prepared.resources.peak_offscreen_bytes {
            return Err(format!(
                "d2raster: internal band offscreen peak {} exceeds preflight plan {}",
                scratch.offscreen.peak, prepared.resources.peak_offscreen_bytes
            ));
        }
        let conversion = conversion_bounds(&prepared, canvas.rect);
        let opaque = prepared.background.is_some_and(|b| b.a == 0xff);
        let band = rgba_to_nrgba(canvas, conversion, opaque);
        consume(&band)?;
        top = bottom;
    }
    scratch.release_pattern_tiles();
    scratch.offscreen.release(reservation);
    Ok(())
}

// --------------------------------------------------------------- preflight

pub struct Preflight<'d> {
    pub(crate) document: &'d Document,
    pub(crate) options: FrameOptions,
    pub(crate) frame_bounds: Rect,
    pub(crate) nodes: i64,
    pub(crate) path_segments: i64,
    pub(crate) text_runes: i64,
    pub(crate) text_coverage_checks: i64,
    pub(crate) text_shaping_runs: i64,
    pub(crate) shaped_glyphs: i64,
    pub(crate) shaping_workspace: d2_rs_fontface::ShapingWorkspace,
    animation_tracks: i64,
    animation_keyframes: i64,
    filters: i64,
    active: HashSet<usize>,
    active_assets: HashSet<String>,
    pub(crate) fonts: HashMap<String, Rc<PreparedFont>>,
    pub(crate) rasters: HashMap<String, Rc<PreparedRasterAsset>>,
    pub(crate) vectors: HashMap<String, (SceneBox, &'d Node)>,
    pattern_tiles: HashMap<PatternKey, Option<Rc<PatternTile>>>,
    asset_bytes: i64,
    decoded_bytes: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PatternKey {
    root: usize,
    width: i64,
    height: i64,
    scale_x: u64,
    scale_y: u64,
    import_depth: i64,
}

pub fn prepare(document: &Document, options: &FrameOptions, band_height: i64) -> Result<PreparedDocument, String> {
    let mut options = options.clone();
    if options.max_text_runes_per_run == 0 {
        options.max_text_runes_per_run = options.max_path_commands.min(DEFAULT_MAX_TEXT_RUNES_PER_RUN);
    }
    if options.max_font_faces_per_text == 0 {
        options.max_font_faces_per_text = options.max_assets.min(DEFAULT_MAX_FONT_FACES_PER_TEXT);
    }
    if options.max_text_coverage_checks == 0 {
        options.max_text_coverage_checks = DEFAULT_MAX_TEXT_COVERAGE_CHECKS;
    }
    if options.max_text_shaping_runs == 0 {
        options.max_text_shaping_runs = options.max_path_commands.min(DEFAULT_MAX_TEXT_SHAPING_RUNS);
    }
    if options.max_scanline_work == 0 {
        options.max_scanline_work = DEFAULT_MAX_SCANLINE_WORK;
    }
    validate_options(&options)?;
    let view_box = document.view_box;
    if validate_box(view_box).is_err() || view_box.width == 0.0 || view_box.height == 0.0 {
        return Err("d2raster: invalid viewbox".into());
    }
    if !finite(document.logical_width) || !finite(document.logical_height) || document.logical_width <= 0.0 || document.logical_height <= 0.0 {
        return Err("d2raster: invalid logical dimensions".into());
    }
    if document.viewport_fit.0 > ViewportFit::MEET.0 {
        return Err(format!("d2raster: invalid viewport fit {}", document.viewport_fit.0));
    }
    if document.viewport_align.0 > ViewportAlign::X_MID_Y_MID.0 {
        return Err(format!("d2raster: invalid viewport alignment {}", document.viewport_align.0));
    }
    let width_float = document.logical_width * options.scale;
    let height_float = document.logical_height * options.scale;
    if !finite(width_float) || !finite(height_float) || width_float <= 0.0 || height_float <= 0.0 {
        return Err("d2raster: invalid scaled dimensions".into());
    }
    let width_ceil = width_float.ceil();
    let height_ceil = height_float.ceil();
    if width_ceil > options.max_width as f64 {
        return Err(format!("d2raster: frame width {width_ceil:.0} exceeds limit {}", options.max_width));
    }
    if height_ceil > options.max_height as f64 {
        return Err(format!("d2raster: frame height {height_ceil:.0} exceeds limit {}", options.max_height));
    }
    if width_ceil >= i64::MAX as f64 || height_ceil >= i64::MAX as f64 {
        return Err("d2raster: scaled dimensions exceed the platform integer domain".into());
    }
    let width = width_ceil as i64;
    let height = height_ceil as i64;
    if width > options.max_pixels / height {
        return Err(format!("d2raster: frame pixels exceed limit {}", options.max_pixels));
    }
    let storage_height = if band_height > 0 { height.min(band_height) } else { height };
    final_frame_storage_bytes(width, storage_height)?;
    let Some(root_node) = document.root.as_deref() else {
        return Err("d2raster: document has no root node".into());
    };

    let mut sx = width_float / view_box.width;
    let mut sy = height_float / view_box.height;
    let (mut offset_x, mut offset_y) = (0.0, 0.0);
    if document.viewport_fit == ViewportFit::MEET {
        let uniform_scale = d2_rs_scene::go_min(width as f64 / view_box.width, height as f64 / view_box.height);
        sx = uniform_scale;
        sy = uniform_scale;
        if document.viewport_align == ViewportAlign::X_MID_Y_MID {
            // ARM64 raster.go:693–694: FMSUBD.
            offset_x = go_fmsub(view_box.width, uniform_scale, width as f64) / 2.0;
            offset_y = go_fmsub(view_box.height, uniform_scale, height as f64) / 2.0;
        }
    }
    // ARM64 raster.go:700–701: FMSUBD.
    let view_to_pixel = Matrix { a: sx, b: 0.0, c: 0.0, d: sy, e: go_fmsub(view_box.x, sx, offset_x), f: go_fmsub(view_box.y, sy, offset_y) };
    if !view_to_pixel.is_finite() {
        return Err("d2raster: viewbox mapping is non-finite".into());
    }
    let mut p = Preflight {
        document,
        options: options.clone(),
        frame_bounds: Rect::new(0, 0, width, height),
        nodes: 0,
        path_segments: 0,
        text_runes: 0,
        text_coverage_checks: 0,
        text_shaping_runs: 0,
        shaped_glyphs: 0,
        shaping_workspace: d2_rs_fontface::ShapingWorkspace::default(),
        animation_tracks: 0,
        animation_keyframes: 0,
        filters: 0,
        active: HashSet::new(),
        active_assets: HashSet::new(),
        fonts: HashMap::new(),
        rasters: HashMap::new(),
        vectors: HashMap::new(),
        pattern_tiles: HashMap::new(),
        asset_bytes: 0,
        decoded_bytes: 0,
    };
    p.assets()?;
    let mut root = p.node(root_node, view_to_pixel, 1, 0)?;
    let frame_bounds = p.frame_bounds;
    let patterns = collect_prepared_patterns(&root, frame_bounds)?;
    let resources = if band_height > 0 {
        set_band_reference_bounds(&mut root, frame_bounds);
        for pattern in &patterns {
            set_band_reference_bounds(&mut pattern.root.borrow_mut(), pattern.bounds);
        }
        plan_band_resources(&root, frame_bounds, &patterns, &options, band_height)?
    } else {
        plan_render_resources(Some(&root), frame_bounds, &patterns, options.max_offscreen_bytes)?
    };
    if resources.peak_offscreen_bytes > options.max_offscreen_bytes {
        return Err(format!(
            "d2raster: peak offscreen pixel storage {} bytes exceeds limit {}",
            resources.peak_offscreen_bytes, options.max_offscreen_bytes
        ));
    }
    if resources.even_odd_clip_work > options.max_even_odd_clip_work {
        return Err(format!("d2raster: even-odd clip work {} exceeds limit {}", resources.even_odd_clip_work, options.max_even_odd_clip_work));
    }
    if resources.scanline_work > options.max_scanline_work {
        return Err(format!("d2raster: scanline work {} exceeds limit {}", resources.scanline_work, options.max_scanline_work));
    }
    Ok(PreparedDocument { width, height, background: options.background, root, patterns, resources })
}

fn final_frame_storage_bytes(width: i64, height: i64) -> Result<i64, String> {
    let pixels = crate::effects::checked_multiply(width, height).ok_or("d2raster: final frame storage exceeds the int64 domain")?;
    crate::effects::checked_multiply(pixels, 4).ok_or_else(|| "d2raster: final frame storage exceeds the platform integer domain".to_owned())
}

fn validate_options(options: &FrameOptions) -> Result<(), String> {
    if !finite(options.scale) || options.scale <= 0.0 {
        return Err("d2raster: scale must be finite and positive".into());
    }
    if options.time < 0 {
        return Err("d2raster: frame time must not be negative".into());
    }
    let limits = [
        options.max_width,
        options.max_height,
        options.max_pixels,
        options.max_nodes,
        options.max_depth,
        options.max_path_commands,
        options.max_text_runes_per_run,
        options.max_font_faces_per_text,
        options.max_text_coverage_checks,
        options.max_text_shaping_runs,
        options.max_animation_tracks,
        options.max_animation_keyframes,
        options.max_assets,
        options.max_asset_bytes,
        options.max_decoded_asset_bytes,
        options.max_import_depth,
        options.max_offscreen_bytes,
        options.max_even_odd_clip_work,
        options.max_scanline_work,
    ];
    if limits.iter().any(|limit| *limit <= 0) {
        return Err("d2raster: every frame resource limit must be positive".into());
    }
    Ok(())
}

pub fn validate_box(b: SceneBox) -> Result<(), String> {
    if !finite(b.x) || !finite(b.y) || !finite(b.width) || !finite(b.height) {
        return Err("non-finite box".into());
    }
    if b.width < 0.0 || b.height < 0.0 {
        return Err("negative box size".into());
    }
    Ok(())
}

pub fn validate_raster_point(point: Point) -> Result<(), String> {
    if !finite_point(point) || point.x.abs() > f32::MAX as f64 || point.y.abs() > f32::MAX as f64 {
        return Err("point is outside the finite float32 raster domain".into());
    }
    Ok(())
}

fn validate_stroke_centerline(path: &Subpath) -> Result<(), String> {
    let points = clean_points(&path.points, path.closed);
    let edge_count = if path.closed && points.len() > 1 { points.len() } else { points.len().saturating_sub(1) };
    for edge in 0..edge_count {
        let start = points[edge];
        let end = points[(edge + 1) % points.len()];
        let (dx, dy) = (end.x - start.x, end.y - start.y);
        if !finite(dx) || !finite(dy) || !finite(crate::fma::gomath::hypot(dx, dy)) {
            return Err("centerline delta is non-finite".into());
        }
    }
    Ok(())
}

/// Go's `%T` of a d2scene asset value.
pub fn asset_go_type(asset: &Asset) -> &'static str {
    match asset {
        Asset::FontAsset { .. } => "d2scene.FontAsset",
        Asset::RasterAsset { .. } => "d2scene.RasterAsset",
        Asset::VectorAsset { .. } => "d2scene.VectorAsset",
    }
}

/// Go's `%g` for the few float diagnostics that print one.
pub fn go_g(value: f64) -> String {
    let formatted = format!("{value}");
    let exponent = if value == 0.0 { 0 } else { value.abs().log10().floor() as i32 };
    if value.is_finite() && (exponent < -4 || exponent >= 21) {
        let text = format!("{value:e}");
        let (mantissa, power) = text.split_once('e').unwrap_or((&text, "0"));
        let power: i32 = power.parse().unwrap_or(0);
        return format!("{mantissa}e{}{:02}", if power < 0 { '-' } else { '+' }, power.abs());
    }
    formatted
}

impl<'d> Preflight<'d> {
    pub(crate) fn preparation_bounds(&self) -> Rect {
        let max_platform = i64::MAX;
        let min_platform = i64::MIN;
        let margin = max_platform / 4;
        let f = self.frame_bounds;
        let min_x = if f.min.x >= min_platform + margin { f.min.x - margin } else { min_platform };
        let min_y = if f.min.y >= min_platform + margin { f.min.y - margin } else { min_platform };
        let max_x = if f.max.x <= max_platform - margin { f.max.x + margin } else { max_platform };
        let max_y = if f.max.y <= max_platform - margin { f.max.y + margin } else { max_platform };
        Rect::new(min_x, min_y, max_x, max_y)
    }

    fn assets(&mut self) -> Result<(), String> {
        let assets = &self.document.assets;
        if assets.len() as i64 > self.options.max_assets {
            return Err(format!("d2raster: asset count {} exceeds limit {}", assets.len(), self.options.max_assets));
        }
        let ids: Vec<String> = assets.keys().cloned().collect();
        for id in &ids {
            if id.is_empty() {
                return Err("d2raster: empty asset ID".into());
            }
            match &assets[id] {
                Asset::FontAsset { data, face_index, .. } => {
                    self.add_asset_bytes(id, data.len() as i64)?;
                    let font = PreparedFont::parse(data, *face_index).map_err(|e| format!("d2raster: font asset {id:?}: {e}"))?;
                    self.fonts.insert(id.clone(), Rc::new(font));
                }
                Asset::RasterAsset { mime_type, data, pixel_width, pixel_height, decoded_bytes } => {
                    self.add_asset_bytes(id, data.len() as i64)?;
                    let available = self.options.max_decoded_asset_bytes - self.decoded_bytes;
                    let (prepared, decoded) = prepare_raster_asset(id, mime_type, data, *pixel_width, *pixel_height, *decoded_bytes, available)?;
                    self.decoded_bytes += decoded;
                    self.rasters.insert(id.clone(), Rc::new(prepared));
                }
                Asset::VectorAsset { view_box, root } => {
                    if validate_box(*view_box).is_err() || view_box.width == 0.0 || view_box.height == 0.0 {
                        return Err(format!("d2raster: vector asset {id:?} has invalid viewbox"));
                    }
                    let Some(root) = root.as_deref() else {
                        return Err(format!("d2raster: vector asset {id:?} has no root node"));
                    };
                    self.vectors.insert(id.clone(), (*view_box, root));
                }
            }
        }
        self.validate_retained_vector_assets(&ids)
    }

    fn add_asset_bytes(&mut self, id: &str, encoded: i64) -> Result<(), String> {
        if encoded > self.options.max_asset_bytes - self.asset_bytes {
            return Err(format!("d2raster: asset {id:?} causes retained asset bytes to exceed limit {}", self.options.max_asset_bytes));
        }
        self.asset_bytes += encoded;
        Ok(())
    }

    pub(crate) fn add_prepared_nodes(&mut self, count: i64) -> Result<(), String> {
        if count <= 0 {
            return Err("d2raster: internal prepared node charge must be positive".into());
        }
        if count > self.options.max_nodes - self.nodes {
            return Err(format!("d2raster: node count exceeds limit {}", self.options.max_nodes));
        }
        self.nodes += count;
        Ok(())
    }

    pub(crate) fn add_path_segment(&mut self) -> Result<(), String> {
        self.path_segments += 1;
        if self.path_segments > self.options.max_path_commands {
            return Err(format!("path command count exceeds limit {}", self.options.max_path_commands));
        }
        Ok(())
    }

    pub(crate) fn charge_animation_work(&mut self, node_id: &str, tracks: &[d2_rs_scene::Track]) -> Result<(), String> {
        if tracks.len() as i64 > self.options.max_animation_tracks - self.animation_tracks {
            return Err(format!(
                "d2raster: node {node_id:?} causes animation track count to exceed limit {}",
                self.options.max_animation_tracks
            ));
        }
        self.animation_tracks += tracks.len() as i64;
        for (track_index, track) in tracks.iter().enumerate() {
            let keyframes = track.keyframes.as_deref().map_or(0, |k| k.len()) as i64;
            if keyframes > self.options.max_animation_keyframes - self.animation_keyframes {
                return Err(format!(
                    "d2raster: node {node_id:?} animation {track_index} causes keyframe count to exceed limit {}",
                    self.options.max_animation_keyframes
                ));
            }
            self.animation_keyframes += keyframes;
        }
        Ok(())
    }

    pub(crate) fn charge_filter_work(&mut self, node_id: &str, filters: usize) -> Result<(), String> {
        if filters as i64 > self.options.max_nodes - self.filters {
            return Err(format!("d2raster: node {node_id:?} causes filter count to exceed structural limit {}", self.options.max_nodes));
        }
        self.filters += filters as i64;
        Ok(())
    }

    fn node(&mut self, node: &'d Node, parent: Matrix, depth: i64, import_depth: i64) -> Result<PreparedNode, String> {
        if depth > self.options.max_depth {
            return Err(format!("d2raster: node depth {depth} exceeds limit {}", self.options.max_depth));
        }
        let key = node as *const Node as usize;
        if self.active.contains(&key) {
            return Err(format!("d2raster: node cycle at {:?}", node.id));
        }
        self.add_prepared_nodes(1)?;
        if !node.transform.is_finite() || !finite(node.opacity) || node.opacity < 0.0 || node.opacity > 1.0 {
            return Err(format!("d2raster: node {:?} has invalid transform or opacity", node.id));
        }
        if !supported_blend_mode(node.blend) {
            return Err(format!("d2raster: node {:?} uses invalid or unsupported blend mode {}", node.id, node.blend.0));
        }
        let filters = node.filters.as_deref().unwrap_or_default();
        self.charge_filter_work(&node.id, filters.len())?;
        let tracks = node.animations.as_deref().unwrap_or_default();
        self.charge_animation_work(&node.id, tracks)?;
        self.active.insert(key);
        let result = self.node_body(node, parent, depth, import_depth);
        self.active.remove(&key);
        result
    }

    fn node_body(&mut self, node: &'d Node, parent: Matrix, depth: i64, import_depth: i64) -> Result<PreparedNode, String> {
        let mut node_transform = node.transform;
        let mut opacity = node.opacity;
        let mut overrides = AnimationOverrides::default();
        let mut seen = HashSet::new();
        for (track_index, track) in node.animations.as_deref().unwrap_or_default().iter().enumerate() {
            if !seen.insert((track.property.0, track.target_index)) {
                return Err(format!(
                    "d2raster: node {:?} has duplicate animation target property {} index {}",
                    node.id, track.property.0, track.target_index
                ));
            }
            let value = animation_value_at(track, self.options.time).map_err(|e| format!("d2raster: node {:?} animation {track_index}: {e}", node.id))?;
            if track.property != d2_rs_scene::AnimationProperty::DROP_SHADOW && track.target_index != 0 {
                return Err(format!(
                    "d2raster: node {:?} animation {track_index} uses non-zero target index for scalar property {}",
                    node.id, track.property.0
                ));
            }
            use d2_rs_scene::AnimationProperty as A;
            match track.property {
                A::OPACITY => {
                    if value.number < 0.0 || value.number > 1.0 {
                        return Err(format!("d2raster: node {:?} animation {track_index} resolves opacity outside [0,1]", node.id));
                    }
                    opacity = value.number;
                }
                A::TRANSFORM => node_transform = value.transform,
                A::STROKE_DASH_OFFSET => overrides.dash_offset = Some(value.number),
                A::FILL_COLOR => overrides.fill_color = Some(value.color),
                A::STROKE_COLOR => overrides.stroke_color = Some(value.color),
                A::DROP_SHADOW => {
                    overrides.drop_shadows.insert(track.target_index, value.shadow);
                }
                property => return Err(format!("d2raster: node {:?} animation {track_index} uses unknown property {}", node.id, property.0)),
            }
        }
        let transform = mmul(&parent, &node_transform);
        if !transform.is_finite() {
            return Err(format!("d2raster: node {:?} has a non-finite composed transform", node.id));
        }
        let normalized_filters = normalize_node_filters(&node.id, node.filters.as_deref().unwrap_or_default(), &overrides.drop_shadows)?;
        let primitive = self.primitive(&node.id, node.primitive.as_ref(), &transform, &overrides, depth, import_depth)?;
        if primitive.is_none() && (overrides.fill_color.is_some() || overrides.stroke_color.is_some() || overrides.dash_offset.is_some()) {
            return Err(format!("d2raster: node {:?} has a paint or stroke animation but no primitive", node.id));
        }
        let mut prepared = PreparedNode::group(Vec::new(), None);
        prepared.opacity = opacity;
        prepared.blend = node.blend;
        if let Some(primitive) = &primitive {
            prepared.bounds = primitive.bounds;
        }
        prepared.primitive = primitive.map(Box::new);
        if let Some(clip) = &node.clip {
            prepared.clip = Some(self.clip(&node.id, clip, &transform)?);
        }
        if let Some(mask) = &node.mask {
            let Some(mask_root) = mask.root.as_deref().filter(|_| mask.transform.is_finite() && mask.r#type.0 <= MaskType::LUMINANCE.0) else {
                return Err(format!("d2raster: node {:?} has invalid mask", node.id));
            };
            let mask_transform = mmul(&transform, &mask.transform);
            if !mask_transform.is_finite() {
                return Err(format!("d2raster: node {:?} has a non-finite composed mask transform", node.id));
            }
            let root = self.node(mask_root, mask_transform, depth + 1, import_depth).map_err(|e| format!("d2raster: node {:?} mask: {e}", node.id))?;
            prepared.mask = Some(Box::new(PreparedMask { kind: mask.r#type, root }));
        }
        for child in node.children() {
            let prepared_child = self.node(child, transform, depth + 1, import_depth)?;
            prepared.bounds = union_rect(prepared.bounds, prepared_child.bounds);
            prepared.children.push(prepared_child);
        }
        prepared.content_bounds = prepared.bounds;
        let (filters, bounds) = prepare_filters(&node.id, &normalized_filters, &transform, prepared.bounds)?;
        prepared.filters = filters;
        prepared.bounds = bounds;
        if let Some(clip) = &prepared.clip {
            prepared.bounds = prepared.bounds.intersect(clip.bounds);
        }
        if let Some(mask) = &prepared.mask {
            prepared.bounds = prepared.bounds.intersect(mask.root.bounds);
        }
        Ok(prepared)
    }

    fn clip(&mut self, node_id: &str, clip: &Clip, transform: &Matrix) -> Result<PreparedClip, String> {
        if !clip.transform.is_finite() {
            return Err(format!("d2raster: node {node_id:?} has invalid clip transform"));
        }
        if clip.path.fill_rule.0 > FillRule::EVEN_ODD.0 {
            return Err(format!("d2raster: node {node_id:?} clip has invalid fill rule {}", clip.path.fill_rule.0));
        }
        let clip_transform = mmul(transform, &clip.transform);
        if !clip_transform.is_finite() {
            return Err(format!("d2raster: node {node_id:?} has a non-finite composed clip transform"));
        }
        let paths = flatten_scene_path(&clip.path, &clip_transform, &mut || self.add_path_segment()).map_err(|e| format!("d2raster: node {node_id:?} clip: {e}"))?;
        let mut device_paths = Vec::with_capacity(paths.len());
        let mut edges = 0i64;
        for path in &paths {
            let mut points = Vec::with_capacity(path.points.len());
            for point in &path.points {
                let device_point = mpoint(&clip_transform, *point);
                validate_raster_point(device_point).map_err(|e| format!("d2raster: node {node_id:?} clip geometry: {e}"))?;
                points.push(device_point);
            }
            if path.points.len() >= 2 {
                edges += path.points.len() as i64;
            }
            device_paths.push(Subpath { points, closed: path.closed });
        }
        let bounds = subpath_pixel_bounds(&device_paths, &Matrix::identity(), 0.0, self.preparation_bounds());
        Ok(PreparedClip { reference_bounds: Rect::default(), subpaths: device_paths, fill_rule: clip.path.fill_rule, bounds, edges })
    }

    fn primitive(&mut self, node_id: &str, primitive: Option<&'d Primitive>, transform: &Matrix, animation: &AnimationOverrides, depth: i64, import_depth: i64) -> Result<Option<PreparedPrimitive>, String> {
        let Some(primitive) = primitive else {
            return Ok(None);
        };
        Ok(Some(match primitive {
            Primitive::Rect(rect) => self.rect(node_id, rect, transform, animation, import_depth)?,
            Primitive::Ellipse(ellipse) => self.ellipse(node_id, ellipse, transform, animation, import_depth)?,
            Primitive::Path(path) => self.path(node_id, path, transform, animation, import_depth)?,
            Primitive::TextRun(text) => self.text(node_id, text, transform, animation, import_depth)?,
            Primitive::Image(image) => self.image(node_id, image, transform, animation, depth, import_depth)?,
        }))
    }

    fn rect(&mut self, node_id: &str, rect: &'d SceneRect, transform: &Matrix, animation: &AnimationOverrides, import_depth: i64) -> Result<PreparedPrimitive, String> {
        validate_box(rect.r#box).map_err(|e| format!("d2raster: node {node_id:?} rectangle: {e}"))?;
        if !finite(rect.radius_x) || !finite(rect.radius_y) || rect.radius_x < 0.0 || rect.radius_y < 0.0 {
            return Err(format!("d2raster: node {node_id:?} rectangle has invalid corner radius"));
        }
        let subpaths = rounded_rect_subpaths(rect, flatten_tolerance(transform), &mut || self.add_path_segment()).map_err(|e| format!("d2raster: node {node_id:?} rectangle: {e}"))?;
        let object_bounds = rect.r#box;
        let fill = self.prepare_animated_paint(rect.fill.as_ref(), animation.fill_color, object_bounds, transform, import_depth).map_err(|e| format!("d2raster: node {node_id:?} rectangle fill: {e}"))?;
        let stroke = self
            .prepare_animated_stroke(rect.stroke.as_ref(), animation.stroke_color, animation.dash_offset, object_bounds, transform, import_depth)
            .map_err(|e| format!("d2raster: node {node_id:?} rectangle stroke: {e}"))?;
        self.finish_primitive(node_id, subpaths, transform, fill, stroke)
    }

    fn ellipse(&mut self, node_id: &str, ellipse: &'d Ellipse, transform: &Matrix, animation: &AnimationOverrides, import_depth: i64) -> Result<PreparedPrimitive, String> {
        if !finite_point(ellipse.center) || !finite(ellipse.radius_x) || !finite(ellipse.radius_y) || ellipse.radius_x < 0.0 || ellipse.radius_y < 0.0 {
            return Err(format!("d2raster: node {node_id:?} ellipse has invalid geometry"));
        }
        let subpaths = ellipse_subpaths(ellipse.center, ellipse.radius_x, ellipse.radius_y, flatten_tolerance(transform), &mut || self.add_path_segment())
            .map_err(|e| format!("d2raster: node {node_id:?} ellipse: {e}"))?;
        let object_bounds = SceneBox {
            x: ellipse.center.x - ellipse.radius_x,
            y: ellipse.center.y - ellipse.radius_y,
            width: 2.0 * ellipse.radius_x,
            height: 2.0 * ellipse.radius_y,
        };
        let fill = self.prepare_animated_paint(ellipse.fill.as_ref(), animation.fill_color, object_bounds, transform, import_depth).map_err(|e| format!("d2raster: node {node_id:?} ellipse fill: {e}"))?;
        let stroke = self
            .prepare_animated_stroke(ellipse.stroke.as_ref(), animation.stroke_color, animation.dash_offset, object_bounds, transform, import_depth)
            .map_err(|e| format!("d2raster: node {node_id:?} ellipse stroke: {e}"))?;
        self.finish_primitive(node_id, subpaths, transform, fill, stroke)
    }

    fn path(&mut self, node_id: &str, path: &'d Path, transform: &Matrix, animation: &AnimationOverrides, import_depth: i64) -> Result<PreparedPrimitive, String> {
        if path.fill_rule.0 > FillRule::EVEN_ODD.0 {
            return Err(format!("d2raster: node {node_id:?} path has invalid fill rule {}", path.fill_rule.0));
        }
        let subpaths = flatten_scene_path(path, transform, &mut || self.add_path_segment()).map_err(|e| format!("d2raster: node {node_id:?} path: {e}"))?;
        let geometry_bounds = path.geometry_bounds().map_err(|e| format!("d2raster: node {node_id:?} path bounds: {e}"))?;
        let object_bounds = geometry_bounds.to_box();
        let fill = self.prepare_animated_paint(path.fill.as_ref(), animation.fill_color, object_bounds, transform, import_depth).map_err(|e| format!("d2raster: node {node_id:?} path fill: {e}"))?;
        let stroke = self
            .prepare_animated_stroke(path.stroke.as_ref(), animation.stroke_color, animation.dash_offset, object_bounds, transform, import_depth)
            .map_err(|e| format!("d2raster: node {node_id:?} path stroke: {e}"))?;
        let has_fill = fill.is_some();
        let mut prepared = self.finish_primitive(node_id, subpaths, transform, fill, stroke)?;
        prepared.fill_rule = path.fill_rule;
        if path.fill_rule == FillRule::EVEN_ODD && has_fill {
            for subpath in &prepared.subpaths {
                let points = subpath.points.iter().map(|p| mpoint(transform, *p)).collect();
                if subpath.points.len() >= 2 {
                    prepared.even_odd_edges += subpath.points.len() as i64;
                }
                prepared.even_odd_subpaths.push(Subpath { points, closed: subpath.closed });
            }
        }
        Ok(prepared)
    }

    pub(crate) fn finish_primitive(&mut self, node_id: &str, paths: Vec<Subpath>, transform: &Matrix, fill: Option<PreparedPaint>, stroke: Option<PreparedStroke>) -> Result<PreparedPrimitive, String> {
        for path in &paths {
            for point in &path.points {
                validate_raster_point(mpoint(transform, *point)).map_err(|e| format!("d2raster: node {node_id:?} geometry: {e}"))?;
            }
        }
        let mut prepared = PreparedPrimitive { transform: *transform, ..Default::default() };
        if fill.is_some() {
            prepared.bounds = subpath_pixel_bounds(&paths, transform, 0.0, self.preparation_bounds());
        }
        prepared.fill = fill;
        let Some(stroke) = stroke else {
            prepared.subpaths = paths;
            return Ok(prepared);
        };
        if !finite(stroke.width * transform.max_scale()) {
            return Err(format!("d2raster: node {node_id:?} stroke has non-finite transformed width"));
        }
        for path in &paths {
            validate_stroke_centerline(path).map_err(|e| format!("d2raster: node {node_id:?} stroke geometry: {e}"))?;
            let runs = make_stroke_runs(path, &stroke.dashes, stroke.dash_offset, &mut || self.add_path_segment()).map_err(|e| format!("d2raster: node {node_id:?} stroke: {e}"))?;
            prepared.stroke_runs.extend(runs);
        }
        let half_width = stroke.width / 2.0;
        for run in &prepared.stroke_runs {
            for point in &run.points {
                for (ox, oy) in [(-half_width, -half_width), (half_width, -half_width), (half_width, half_width), (-half_width, half_width)] {
                    validate_raster_point(mpoint(transform, Point::new(point.x + ox, point.y + oy))).map_err(|e| format!("d2raster: node {node_id:?} stroke geometry: {e}"))?;
                }
            }
            if stroke.join != LineJoin::ROUND {
                let (join_start, join_end) = if run.closed { (0, run.points.len()) } else { (1, run.points.len() - 1) };
                let previous_index = if join_start == 0 { run.points.len() - 1 } else { join_start - 1 };
                let mut incoming = unit_vector(run.points[previous_index], run.points[join_start % run.points.len()]);
                for index in join_start..join_end {
                    let next_index = if index + 1 == run.points.len() { 0 } else { index + 1 };
                    let vertex = run.points[index];
                    let outgoing = unit_vector(vertex, run.points[next_index]);
                    let mut polygon = [Point::default(); 4];
                    let count = stroke_join_polygon_from_units(&mut polygon, vertex, incoming, outgoing, half_width, stroke.join, stroke.miter_limit);
                    for point in &polygon[..count] {
                        validate_raster_point(mpoint(transform, *point)).map_err(|e| format!("d2raster: node {node_id:?} stroke join geometry: {e}"))?;
                    }
                    incoming = outgoing;
                }
            }
        }
        let expansion = stroke.width * transform.max_scale() / 2.0;
        let mut factor = 1.0;
        if stroke.cap == LineCap::SQUARE {
            factor = std::f64::consts::SQRT_2;
        }
        if stroke.join == LineJoin::MITER && stroke.miter_limit > factor {
            factor = stroke.miter_limit;
        }
        prepared.bounds = union_rect(prepared.bounds, stroke_run_pixel_bounds(&prepared.stroke_runs, transform, expansion * factor, self.preparation_bounds()));
        prepared.subpaths = paths;
        prepared.stroke = Some(stroke);
        Ok(prepared)
    }

    pub(crate) fn prepare_animated_paint(&mut self, paint: Option<&'d Paint>, animated_color: Option<Nrgba>, object_bounds: SceneBox, object_to_device: &Matrix, import_depth: i64) -> Result<Option<PreparedPaint>, String> {
        if let Some(Paint::PatternPaint(pattern)) = paint {
            if animated_color.is_some() {
                return Err("color animation targets pattern paint".into());
            }
            return self.prepare_pattern_paint(pattern, object_bounds, object_to_device, import_depth).map(Some);
        }
        prepare_animated_paint_with_pattern(paint, animated_color, object_bounds, object_to_device, None)
    }

    pub(crate) fn prepare_animated_stroke(
        &mut self,
        stroke: Option<&'d Stroke>,
        animated_color: Option<Nrgba>,
        animated_dash_offset: Option<f64>,
        object_bounds: SceneBox,
        object_to_device: &Matrix,
        import_depth: i64,
    ) -> Result<Option<PreparedStroke>, String> {
        let Some(stroke) = stroke else {
            if animated_color.is_some() || animated_dash_offset.is_some() {
                return Err("stroke animation targets missing stroke".into());
            }
            return Ok(None);
        };
        if !finite(stroke.width) || stroke.width < 0.0 || !finite(stroke.miter_limit) || !finite(stroke.dash_offset) {
            return Err("invalid stroke".into());
        }
        let paint = self.prepare_animated_paint(stroke.paint.as_ref(), animated_color, object_bounds, object_to_device, import_depth)?;
        let dashes = stroke.dashes.as_deref().unwrap_or_default();
        let mut dash_total = 0.0;
        for dash in dashes {
            if !finite(*dash) || *dash <= 0.0 {
                return Err("invalid stroke dash".into());
            }
            dash_total += *dash;
            if !finite(dash_total) {
                return Err("invalid stroke dash total".into());
            }
        }
        let Some(paint) = paint.filter(|_| stroke.width != 0.0) else {
            return Ok(None);
        };
        if !matches!(stroke.cap, LineCap::BUTT | LineCap::ROUND | LineCap::SQUARE) {
            return Err(format!("unsupported line cap {}", stroke.cap.0));
        }
        if !matches!(stroke.join, LineJoin::MITER | LineJoin::ROUND | LineJoin::BEVEL) {
            return Err(format!("unsupported line join {}", stroke.join.0));
        }
        let mut miter_limit = stroke.miter_limit;
        if stroke.join == LineJoin::MITER {
            if miter_limit == 0.0 {
                miter_limit = 4.0;
            }
            if miter_limit < 1.0 {
                return Err(format!("invalid miter limit {}", go_g(stroke.miter_limit)));
            }
        }
        Ok(Some(PreparedStroke {
            reference_bounds: Rect::default(),
            paint,
            width: stroke.width,
            cap: stroke.cap,
            join: stroke.join,
            miter_limit,
            dashes: dashes.to_vec(),
            dash_offset: animated_dash_offset.unwrap_or(stroke.dash_offset),
        }))
    }

    fn prepare_pattern_paint(&mut self, pattern: &'d PatternPaint, object_bounds: SceneBox, object_to_device: &Matrix, import_depth: i64) -> Result<PreparedPaint, String> {
        validate_box(pattern.tile).map_err(|e| format!("pattern tile: {e}"))?;
        if pattern.tile.width == 0.0 || pattern.tile.height == 0.0 {
            return Err("pattern tile has zero width or height".into());
        }
        if !finite(pattern.tile.x + pattern.tile.width) || !finite(pattern.tile.y + pattern.tile.height) {
            return Err("pattern tile endpoints are non-finite".into());
        }
        let Some(root) = pattern.root.as_deref() else {
            return Err("pattern has no root node".into());
        };
        let (pattern_to_device, device_to_pattern) =
            prepare_pattern_transform(pattern.units, &pattern.transform, object_bounds, object_to_device).map_err(|e| format!("pattern: {e}"))?;
        let width = pattern_tile_pixels(mvector(&pattern_to_device, Point::new(pattern.tile.width, 0.0))).map_err(|e| format!("pattern tile width: {e}"))?;
        let height = pattern_tile_pixels(mvector(&pattern_to_device, Point::new(0.0, pattern.tile.height))).map_err(|e| format!("pattern tile height: {e}"))?;
        let bounds = Rect::new(0, 0, width, height);
        let tile_bytes = crate::effects::pixel_storage_bytes(bounds, 4).map_err(|e| format!("pattern tile: {e}"))?;
        let scale_x = width as f64 / pattern.tile.width;
        let scale_y = height as f64 / pattern.tile.height;
        if !finite(scale_x) || !finite(scale_y) || scale_x <= 0.0 || scale_y <= 0.0 {
            return Err("pattern tile cannot be represented in raster coordinates".into());
        }
        let next_import_depth = import_depth + 1;
        let key = PatternKey {
            root: root as *const Node as usize,
            width,
            height,
            scale_x: scale_x.to_bits(),
            scale_y: scale_y.to_bits(),
            import_depth: next_import_depth,
        };
        let tile_resource = match self.pattern_tiles.get(&key) {
            Some(None) => return Err("pattern root: cyclic pattern resource".into()),
            Some(Some(tile)) => Rc::clone(tile),
            None => {
                self.pattern_tiles.insert(key, None);
                let previous_frame_bounds = self.frame_bounds;
                self.frame_bounds = bounds;
                let prepared_root = self.node(root, scale(scale_x, scale_y), 1, next_import_depth);
                self.frame_bounds = previous_frame_bounds;
                let prepared_root = match prepared_root {
                    Ok(root) => root,
                    Err(err) => {
                        self.pattern_tiles.remove(&key);
                        return Err(format!("pattern root: {err}"));
                    }
                };
                let tile = Rc::new(PatternTile {
                    width,
                    height,
                    bounds,
                    tile_bytes,
                    root: std::cell::RefCell::new(prepared_root),
                    image: std::cell::RefCell::new(None),
                    rendering: std::cell::Cell::new(false),
                });
                self.pattern_tiles.insert(key, Some(Rc::clone(&tile)));
                tile
            }
        };
        let (pixel_offset_x, pixel_offset_y, direct_pixels) = direct_pattern_pixel_offsets(pattern.tile, &device_to_pattern, width, height);
        Ok(PreparedPaint {
            kind: PaintKind::Pattern,
            solid: Nrgba::default(),
            gradient: Rc::default(),
            pattern: Some(Rc::new(PreparedPattern {
                tile: pattern.tile,
                origin_mod_x: pattern.tile.x % pattern.tile.width,
                origin_mod_y: pattern.tile.y % pattern.tile.height,
                device_to_pattern,
                tile_resource,
                direct_pixels,
                pixel_offset_x,
                pixel_offset_y,
            })),
        })
    }

    fn image(&mut self, node_id: &str, image: &'d Image, transform: &Matrix, animation: &AnimationOverrides, depth: i64, import_depth: i64) -> Result<PreparedPrimitive, String> {
        if animation.fill_color.is_some() || animation.stroke_color.is_some() || animation.dash_offset.is_some() {
            return Err(format!("d2raster: node {node_id:?} image cannot be targeted by paint or stroke animation"));
        }
        if image.asset.is_empty() {
            return Err(format!("d2raster: node {node_id:?} image has an empty asset ID"));
        }
        validate_box(image.r#box).map_err(|e| format!("d2raster: node {node_id:?} image: {e}"))?;
        if image.aspect.align.0 > AspectAlign::X_MAX_Y_MAX.0 || image.aspect.fit.0 > AspectFit::SLICE.0 {
            return Err(format!(
                "d2raster: node {node_id:?} image has invalid aspect-ratio policy align={} fit={}",
                image.aspect.align.0, image.aspect.fit.0
            ));
        }
        if let Some(asset) = self.rasters.get(&image.asset).cloned() {
            return self.raster_image(node_id, image, transform, asset);
        }
        if let Some((view_box, root)) = self.vectors.get(&image.asset).copied() {
            return self.vector_image(node_id, image, transform, view_box, root, depth, import_depth);
        }
        if let Some(raw) = self.document.assets.get(&image.asset) {
            return Err(format!("d2raster: node {node_id:?} image asset {:?} is not a raster or vector asset (got {})", image.asset, asset_go_type(raw)));
        }
        Err(format!("d2raster: node {node_id:?} image references missing asset {:?}", image.asset))
    }

    fn raster_image(&mut self, node_id: &str, image: &Image, transform: &Matrix, asset: Rc<PreparedRasterAsset>) -> Result<PreparedPrimitive, String> {
        let placement = image_placement(image.r#box, asset.width, asset.bounds.dy(), image.aspect).map_err(|e| format!("d2raster: node {node_id:?} image aspect ratio: {e}"))?;
        let inverse = make_inverse_affine(transform).map_err(|e| format!("d2raster: node {node_id:?} image transform: {e}"))?;
        let mut bounds = transformed_image_pixel_bounds(image.r#box, transform, self.preparation_bounds()).map_err(|e| format!("d2raster: node {node_id:?} image geometry: {e}"))?;
        if inverse.is_none() || image.r#box.width == 0.0 || image.r#box.height == 0.0 {
            bounds = Rect::default();
        }
        let prepared = PreparedImage { asset, r#box: image.r#box, placement, inverse: inverse.unwrap_or(InverseAffine::ZERO), bounds };
        Ok(PreparedPrimitive { image: Some(prepared), bounds, ..Default::default() })
    }

    #[allow(clippy::too_many_arguments)]
    fn vector_image(&mut self, node_id: &str, image: &Image, transform: &Matrix, view_box: SceneBox, root: &'d Node, depth: i64, import_depth: i64) -> Result<PreparedPrimitive, String> {
        let next_import_depth = import_depth + 1;
        if next_import_depth > self.options.max_import_depth {
            return Err(format!(
                "d2raster: node {node_id:?} vector asset {:?} import depth {next_import_depth} exceeds limit {}",
                image.asset, self.options.max_import_depth
            ));
        }
        if self.active_assets.contains(&image.asset) {
            return Err(format!("d2raster: node {node_id:?} has cyclic vector asset reference at {:?}", image.asset));
        }
        let empty_viewport = image.r#box.width == 0.0 || image.r#box.height == 0.0 || !finite_linear_transform_invertible(transform);
        let mut mapping_destination = image.r#box;
        let mut mapping_aspect = image.aspect;
        if empty_viewport {
            mapping_destination = SceneBox { x: 0.0, y: 0.0, width: d2_rs_scene::go_min(view_box.width, 1.0), height: d2_rs_scene::go_min(view_box.height, 1.0) };
            mapping_aspect = AspectRatio { align: AspectAlign::NONE, fit: AspectFit::MEET };
        }
        let mapping = aspect_ratio_matrix(view_box, mapping_destination, mapping_aspect).map_err(|e| format!("d2raster: node {node_id:?} vector asset {:?} aspect ratio: {e}", image.asset))?;
        if !empty_viewport && !finite_linear_transform_invertible(&mapping) {
            return Err(format!(
                "d2raster: node {node_id:?} vector asset {:?} aspect ratio: mapping is singular in the finite numeric domain",
                image.asset
            ));
        }
        let asset_transform = if empty_viewport { mapping } else { mmul(transform, &mapping) };
        if !asset_transform.is_finite() {
            return Err(format!("d2raster: node {node_id:?} vector asset {:?} has a non-finite composed transform", image.asset));
        }
        self.active_assets.insert(image.asset.clone());
        let prepared_root = self.node(root, asset_transform, depth + 1, next_import_depth);
        self.active_assets.remove(&image.asset);
        let prepared_root = prepared_root.map_err(|e| format!("d2raster: node {node_id:?} vector asset {:?}: {e}", image.asset))?;
        let clip = if empty_viewport { PreparedClip::default() } else { self.image_box_clip(node_id, &image.asset, image.r#box, transform)? };
        let bounds = prepared_root.bounds.intersect(clip.bounds);
        let mut viewport = PreparedNode::group(vec![prepared_root], Some(clip));
        viewport.bounds = bounds;
        Ok(PreparedPrimitive { bounds, vector: Some(Box::new(viewport)), ..Default::default() })
    }

    fn image_box_clip(&mut self, node_id: &str, asset_id: &str, r#box: SceneBox, transform: &Matrix) -> Result<PreparedClip, String> {
        let local = [
            Point::new(r#box.x, r#box.y),
            Point::new(r#box.x + r#box.width, r#box.y),
            Point::new(r#box.x + r#box.width, r#box.y + r#box.height),
            Point::new(r#box.x, r#box.y + r#box.height),
        ];
        let mut points = Vec::with_capacity(4);
        for point in local {
            let device = mpoint(transform, point);
            validate_raster_point(device).map_err(|e| format!("d2raster: node {node_id:?} vector asset {asset_id:?} viewport geometry: {e}"))?;
            points.push(device);
        }
        let mut bounds = transformed_image_pixel_bounds(r#box, transform, self.preparation_bounds())
            .map_err(|e| format!("d2raster: node {node_id:?} vector asset {asset_id:?} viewport geometry: {e}"))?;
        if !finite_linear_transform_invertible(transform) || r#box.width == 0.0 || r#box.height == 0.0 {
            bounds = Rect::default();
        }
        Ok(PreparedClip { reference_bounds: Rect::default(), subpaths: vec![Subpath { points, closed: true }], fill_rule: FillRule::NON_ZERO, bounds, edges: 4 })
    }
}

// ----------------------------------------------------------------- drawing

pub fn render_node(dst: &mut Rgba<'_>, node: &PreparedNode, scratch: &mut Scratch) -> Result<(), String> {
    if node.opacity == 0.0 || node.bounds.intersect(dst.bounds()).empty() {
        return Ok(());
    }
    if uses_effect_layer(node) {
        return render_effect_node(dst, node, scratch);
    }
    if let Some(primitive) = &node.primitive {
        if !primitive.bounds.intersect(dst.bounds()).empty() {
            draw_primitive(dst, primitive, scratch)?;
        }
    }
    for child in &node.children {
        render_node(dst, child, scratch)?;
    }
    Ok(())
}

pub fn draw_primitive(dst: &mut Rgba<'_>, primitive: &PreparedPrimitive, scratch: &mut Scratch) -> Result<(), String> {
    if let Some(image) = &primitive.image {
        draw_prepared_image(dst, image);
        return Ok(());
    }
    if let Some(vector) = &primitive.vector {
        return render_node(dst, vector, scratch);
    }
    if let Some(fill) = &primitive.fill {
        if primitive.fill_rule == FillRule::EVEN_ODD {
            let bounds = subpath_pixel_bounds(&primitive.subpaths, &primitive.transform, 0.0, dst.bounds());
            draw_paint_mask(dst, bounds, fill, scratch, "even-odd fill Alpha mask", &mut |mask| {
                crate::effects::rasterize_even_odd_mask(mask, bounds.min, &primitive.even_odd_subpaths);
                Ok(())
            })?;
        } else if fill.kind == PaintKind::Solid {
            let region_filters = scratch.region_filters;
            let dst_bounds = dst.bounds();
            let (rasterizer, budget) = scratch.reset_with_budget(dst_bounds);
            let origin = set_rasterizer_reference(rasterizer, dst_bounds.min, primitive.reference_bounds.min, region_filters);
            let transform = shifted(origin, &primitive.transform);
            for path in &primitive.subpaths {
                add_fill_subpath(rasterizer, path, &transform);
            }
            rasterizer.draw_rgba(budget, dst, fill.solid).map_err(|e| format!("d2raster: solid fill: {e}"))?;
        } else {
            let bounds = subpath_pixel_bounds(&primitive.subpaths, &primitive.transform, 0.0, dst.bounds());
            draw_rasterized_paint(dst, bounds, fill, scratch, "gradient fill", &mut |rasterizer: &mut Rasterizer, region_filters: bool| {
                let reference = if region_filters {
                    subpath_pixel_bounds(&primitive.subpaths, &primitive.transform, 0.0, primitive.reference_bounds).min
                } else {
                    bounds.min
                };
                let origin = set_rasterizer_reference(rasterizer, bounds.min, reference, region_filters);
                let transform = shifted(origin, &primitive.transform);
                for path in &primitive.subpaths {
                    add_fill_subpath(rasterizer, path, &transform);
                }
            })?;
        }
    }
    if let Some(stroke) = &primitive.stroke {
        draw_stroke(dst, &primitive.stroke_runs, &primitive.transform, stroke, scratch)?;
    }
    Ok(())
}

fn draw_stroke(dst: &mut Rgba<'_>, runs: &[StrokeRun], transform: &Matrix, stroke: &PreparedStroke, scratch: &mut Scratch) -> Result<(), String> {
    if stroke.paint.kind == PaintKind::Solid {
        let region_filters = scratch.region_filters;
        let dst_bounds = dst.bounds();
        let (rasterizer, budget) = scratch.reset_with_budget(dst_bounds);
        let origin = set_rasterizer_reference(rasterizer, dst_bounds.min, stroke.reference_bounds.min, region_filters);
        let shifted_transform = shifted(origin, transform);
        for run in runs {
            add_stroke_run(rasterizer, run, &shifted_transform, stroke);
        }
        return rasterizer.draw_rgba(budget, dst, stroke.paint.solid).map_err(|e| format!("d2raster: solid stroke: {e}"));
    }
    let bounds = painted_stroke_pixel_bounds(runs, transform, stroke, dst.bounds());
    draw_rasterized_paint(dst, bounds, &stroke.paint, scratch, "gradient stroke", &mut |rasterizer: &mut Rasterizer, region_filters: bool| {
        let reference = if region_filters { painted_stroke_pixel_bounds(runs, transform, stroke, stroke.reference_bounds).min } else { bounds.min };
        let origin = set_rasterizer_reference(rasterizer, bounds.min, reference, region_filters);
        let shifted_transform = shifted(origin, transform);
        for run in runs {
            add_stroke_run(rasterizer, run, &shifted_transform, stroke);
        }
    })
}

// --------------------------------------------------------------- patterns

fn collect_prepared_patterns(root: &PreparedNode, dst: Rect) -> Result<Vec<Rc<PatternTile>>, String> {
    let mut collector = PatternCollector { state: HashMap::new(), patterns: Vec::new() };
    collector.node(root, dst)?;
    Ok(collector.patterns)
}

struct PatternCollector {
    state: HashMap<usize, u8>,
    patterns: Vec<Rc<PatternTile>>,
}

impl PatternCollector {
    fn node(&mut self, node: &PreparedNode, dst: Rect) -> Result<(), String> {
        if node.opacity == 0.0 {
            return Ok(());
        }
        let bounds = node.bounds.intersect(dst);
        if bounds.empty() {
            return Ok(());
        }
        let mut paint_bounds = dst;
        if !node.filters.is_empty() {
            paint_bounds = node.content_bounds;
        } else if node.opacity < 1.0 || node.blend != BlendMode::NORMAL || node.clip.is_some() || node.mask.is_some() {
            paint_bounds = bounds;
        }
        if let Some(primitive) = &node.primitive {
            if !primitive.bounds.intersect(paint_bounds).empty() {
                self.primitive(primitive, paint_bounds)?;
            }
        }
        for child in &node.children {
            self.node(child, paint_bounds)?;
        }
        if let Some(mask) = &node.mask {
            self.node(&mask.root, bounds)?;
        }
        Ok(())
    }

    fn primitive(&mut self, primitive: &PreparedPrimitive, dst: Rect) -> Result<(), String> {
        if primitive.image.is_some() {
            return Ok(());
        }
        if let Some(vector) = &primitive.vector {
            return self.node(vector, dst);
        }
        if let Some(fill) = &primitive.fill {
            if !subpath_pixel_bounds(&primitive.subpaths, &primitive.transform, 0.0, dst).empty() {
                self.paint(fill)?;
            }
        }
        if let Some(stroke) = &primitive.stroke {
            if !painted_stroke_pixel_bounds(&primitive.stroke_runs, &primitive.transform, stroke, dst).empty() {
                self.paint(&stroke.paint)?;
            }
        }
        Ok(())
    }

    fn paint(&mut self, paint: &PreparedPaint) -> Result<(), String> {
        if paint.kind != PaintKind::Pattern {
            return Ok(());
        }
        let Some(pattern) = &paint.pattern else {
            return Err("d2raster: internal nil prepared pattern".into());
        };
        let tile = &pattern.tile_resource;
        let key = Rc::as_ptr(tile) as usize;
        match self.state.get(&key) {
            Some(1) => return Err("d2raster: cyclic prepared pattern graph".into()),
            Some(2) => return Ok(()),
            _ => {}
        }
        self.state.insert(key, 1);
        self.node(&tile.root.borrow(), tile.bounds)?;
        self.state.insert(key, 2);
        self.patterns.push(Rc::clone(tile));
        Ok(())
    }
}

// ------------------------------------------------------------------ bands

fn plan_band_resources(root: &PreparedNode, bounds: Rect, patterns: &[Rc<PatternTile>], options: &FrameOptions, band_height: i64) -> Result<RenderResources, String> {
    const PEAK: &str = "d2raster: peak offscreen pixel storage exceeds the int64 domain";
    let mut resources = plan_render_resources_region(None, Rect::default(), patterns, options.max_offscreen_bytes, true)?;
    let mut pixel_peak = resources.peak_offscreen_bytes - resources.rasterizer_bytes;
    let mut pattern_bytes = 0i64;
    for pattern in patterns {
        pattern_bytes = crate::effects::checked_add(pattern_bytes, pattern.tile_bytes).ok_or("d2raster: pattern tile storage exceeds the int64 domain")?;
    }
    let mut top = bounds.min.y;
    while top < bounds.max.y {
        let bottom = top + band_height.min(bounds.max.y - top);
        let band = Rect::new(bounds.min.x, top, bounds.max.x, bottom);
        let band_resources = plan_render_resources_region(Some(root), band, &[], options.max_offscreen_bytes, true)?;
        let band_pixel_peak = crate::effects::checked_add(pattern_bytes, band_resources.peak_offscreen_bytes - band_resources.rasterizer_bytes).ok_or(PEAK)?;
        pixel_peak = pixel_peak.max(band_pixel_peak);
        crate::effects::merge_rasterizer_requirements(&mut resources, &band_resources);
        resources.scanline_work = crate::effects::checked_add(resources.scanline_work, band_resources.scanline_work).ok_or("d2raster: scanline work exceeds the int64 domain")?;
        resources.even_odd_clip_work =
            crate::effects::checked_add(resources.even_odd_clip_work, band_resources.even_odd_clip_work).ok_or("d2raster: even-odd clip work exceeds the int64 domain")?;
        resources.peak_offscreen_bytes = crate::effects::checked_add(pixel_peak, resources.rasterizer_bytes).ok_or(PEAK)?;
        if resources.peak_offscreen_bytes > options.max_offscreen_bytes {
            return Err(format!(
                "d2raster: peak offscreen pixel storage {} bytes exceeds limit {}",
                resources.peak_offscreen_bytes, options.max_offscreen_bytes
            ));
        }
        if resources.even_odd_clip_work > options.max_even_odd_clip_work {
            return Err(format!("d2raster: even-odd clip work {} exceeds limit {}", resources.even_odd_clip_work, options.max_even_odd_clip_work));
        }
        if resources.scanline_work > options.max_scanline_work {
            return Err(format!("d2raster: scanline work {} exceeds limit {}", resources.scanline_work, options.max_scanline_work));
        }
        top = bottom;
    }
    Ok(resources)
}

/// setBandReferenceBounds records full-render canvas origins.
fn set_band_reference_bounds(node: &mut PreparedNode, dst: Rect) {
    if node.opacity == 0.0 {
        return;
    }
    let visible = node.bounds.intersect(dst);
    if visible.empty() {
        return;
    }
    let mut paint_bounds = dst;
    if !node.filters.is_empty() {
        paint_bounds = node.content_bounds;
    } else if node.isolated || node.opacity < 1.0 || node.blend != BlendMode::NORMAL || node.clip.is_some() || node.mask.is_some() {
        paint_bounds = visible;
    }
    if let Some(primitive) = &mut node.primitive {
        primitive.reference_bounds = paint_bounds;
        if let Some(stroke) = &mut primitive.stroke {
            stroke.reference_bounds = paint_bounds;
        }
        if let Some(vector) = &mut primitive.vector {
            set_band_reference_bounds(vector, paint_bounds);
        }
    }
    for child in &mut node.children {
        set_band_reference_bounds(child, paint_bounds);
    }
    if let Some(clip) = &mut node.clip {
        clip.reference_bounds = visible;
    }
    if let Some(mask) = &mut node.mask {
        set_band_reference_bounds(&mut mask.root, visible);
    }
}
