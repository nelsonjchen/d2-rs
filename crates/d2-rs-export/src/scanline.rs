//! Small analytic coverage rasterizer for the ordinary solid-vector subset.
//!
//! D2 v0.9.0's PNG path does not rasterize the SVG with a browser renderer. It
//! feeds flattened paths to a non-zero winding scanline kernel.  This module
//! mirrors that kernel's coverage arithmetic for simple solid paths.  Complex
//! scenes continue through the resvg fallback until their scene resources are
//! translated as well.

// These helpers intentionally preserve the upstream operation shape and
// fixed-point/float arithmetic used by the parity-sensitive raster path.
#![allow(
    clippy::chunks_exact_to_as_chunks,
    clippy::collapsible_if,
    clippy::let_and_return,
    clippy::manual_is_multiple_of,
    clippy::question_mark,
    clippy::too_many_arguments,
    clippy::useless_conversion
)]

#[path = "colr_paint.rs"]
mod colr_paint;
#[path = "native_gradient.rs"]
mod native_gradient;
#[path = "pattern.rs"]
mod pattern;
#[path = "zeroed_layer.rs"]
mod zeroed_layer;
use crate::svg_precision::{NativeRect, SvgImagePathSources, SvgPath, SvgPathCommand, SvgPoint};
use pattern::PatternInstances;
use resvg::tiny_skia::{Path, PathSegment, Point, Transform};
use resvg::usvg::{self, FillRule, Paint, Tree};
use std::collections::BTreeMap;
use std::collections::HashMap;
#[cfg(feature = "diagnostic-traces")]
use std::io::Write;
use ttf_parser::OutlineBuilder;
use zeroed_layer::ZeroedLayer;

// d2raster's scanline kernel stops at depth 18; its separate scene geometry
// helpers use depth 24. Keep the scanline cap independent from those helpers.
const MAX_CURVE_DEPTH: usize = 24;
const MAX_SCANLINE_CURVE_DEPTH: usize = 18;

#[derive(Clone, Copy)]
struct Edge {
    from: Point64,
    to: Point64,
    // Leaf control-hull y range of the cubic that produced this edge. Source
    // flattenCube drops a (sub)cubic whose hull lies wholly above or below
    // the rasterizer, so the edge exists only for windows its hull reaches.
    hull_y: Option<(f64, f64)>,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Point64 {
    x: f64,
    y: f64,
}

fn clean_stroke_points(points: &[Point64], closed: bool) -> Vec<Point64> {
    let first_duplicate = points.windows(2).position(|pair| pair[0] == pair[1]);
    let mut cleaned = if let Some(index) = first_duplicate {
        let mut cleaned = Vec::with_capacity(points.len());
        cleaned.extend_from_slice(&points[..=index]);
        for point in &points[index + 1..] {
            if cleaned.last() != Some(point) {
                cleaned.push(*point);
            }
        }
        cleaned
    } else {
        points.to_vec()
    };
    if closed && cleaned.len() > 1 && cleaned.first() == cleaned.last() {
        cleaned.pop();
    }
    cleaned
}

#[derive(Clone, Copy)]
struct Affine64 {
    sx: f64,
    kx: f64,
    ky: f64,
    sy: f64,
    tx: f64,
    ty: f64,
}

struct StrokeRun {
    points: Vec<Point64>,
    closed: bool,
}

#[derive(Clone, Copy)]
struct ScanEdge {
    end: usize,
    next: usize,
    x: f64,
    y: f64,
    bottom: f64,
    slope: f64,
    inverse_slope: f64,
    winding: f64,
}

/// d2cli raster_png_output.go pngBandHeight.
const GO_PNG_BAND_HEIGHT: usize = 256;

/// Full-frame emulation of d2raster.RenderBands' destinations. `layers` is a
/// stack of effect-layer windows (one per band, absolute surface pixels) for
/// the effect groups currently being drawn.
struct GoBandFrame {
    width: usize,
    height: usize,
    layers: Vec<std::rc::Rc<GoLayer>>,
}

/// One emulated effect node: its per-band layer windows and the reference
/// bounds setBandReferenceBounds gives its primitives (node.bounds within
/// the enclosing effect's reference bounds).
struct GoLayer {
    // Most primitives reach only one band. Do not retain an empty slot for
    // every other band of a tall frame.
    windows: Vec<(usize, (isize, isize, isize, isize))>,
    reference_bounds: Option<(isize, isize, isize, isize)>,
}

impl GoLayer {
    fn window(&self, band: usize) -> Option<(isize, isize, isize, isize)> {
        self.windows
            .binary_search_by_key(&band, |&(band, _)| band)
            .ok()
            .map(|index| self.windows[index].1)
    }

    fn outside_rows(&self, rows: Option<(usize, usize)>) -> bool {
        // Native sharp filters draw through a separate cropped source surface;
        // their leaf recorder may not supply complete-frame geometry. Only
        // qualified prepared bounds can establish that a group is invisible.
        self.reference_bounds.is_some() && rows.is_some() && self.windows_for_rows(rows).is_empty()
    }

    fn windows_for_rows(
        &self,
        rows: Option<(usize, usize)>,
    ) -> &[(usize, (isize, isize, isize, isize))] {
        let Some((first, end)) = rows else {
            return &self.windows;
        };
        // effectRenderBounds intersects each result with its destination;
        // consecutive band windows therefore have disjoint, ordered rows.
        let start = self
            .windows
            .partition_point(|&(_, (_, _, _, bottom))| bottom <= first as isize);
        let end = self
            .windows
            .partition_point(|&(_, (_, top, _, _))| top < end as isize);
        &self.windows[start..end]
    }
}

/// The rounding/reference origin of a primitive drawn inside the innermost
/// emulated effect node, when band emulation applies to this surface.
fn go_layer_reference(width: usize, height: usize) -> Option<Point64> {
    GO_BAND_FRAME.with(|frame| {
        let frame = frame.borrow();
        let frame = frame.as_ref()?;
        if frame.width != width || frame.height != height {
            return None;
        }
        let bounds = frame.layers.last()?.reference_bounds?;
        Some(Point64 {
            x: bounds.0 as f64,
            y: bounds.1 as f64,
        })
    })
}

thread_local! {
    static GO_BAND_FRAME: std::cell::RefCell<Option<GoBandFrame>> =
        const { std::cell::RefCell::new(None) };
    static GO_PNG_BANDS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks a PNG export (d2cli's RenderBands path) while it rasterizes.
pub(crate) struct GoPngBandsScope(bool);

impl GoPngBandsScope {
    pub(crate) fn new() -> Self {
        Self(GO_PNG_BANDS.with(|flag| flag.replace(true)))
    }
}

impl Drop for GoPngBandsScope {
    fn drop(&mut self) {
        GO_PNG_BANDS.with(|flag| flag.set(self.0));
    }
}

fn go_band_frame_active(width: usize, height: usize) -> bool {
    GO_BAND_FRAME.with(|frame| {
        frame
            .borrow()
            .as_ref()
            .is_some_and(|frame| frame.width == width && frame.height == height)
    })
}

/// The source destinations a full-surface rasterizer paints through, or None
/// when band emulation does not apply to this surface.
fn go_band_windows(width: usize, height: usize) -> Option<Vec<(isize, isize, isize, isize)>> {
    GO_BAND_FRAME.with(|frame| {
        let frame = frame.borrow();
        let frame = frame.as_ref()?;
        if frame.width != width || frame.height != height {
            return None;
        }
        // Root paints retain their ordinary reference-anchored surface.
        // Suspending the frame band exposes all destinations for filter input.
        let layer = frame.layers.last()?;
        Some(
            layer
                .windows_for_rows(frame_band_rows(width, height))
                .iter()
                .map(|&(_, window)| window)
                .collect(),
        )
    })
}

/// The pixels of an effect layer that its content can paint: one span of
/// columns per surface row, stored row after row in a compact buffer.
struct LayerRegion {
    // First surface row of `rows`.
    top: usize,
    // Per row: first column, column count, and compact buffer offset.
    rows: Vec<(usize, usize, usize)>,
    len: usize,
    // Built from tracked paint bounds (LayerPaintScope), so the layer is
    // known to be transparent outside it.
    tracked: bool,
}

impl LayerRegion {
    fn empty() -> Self {
        Self {
            top: 0,
            rows: Vec::new(),
            len: 0,
            tracked: false,
        }
    }

    fn full(width: usize, height: usize) -> Self {
        Self {
            top: 0,
            rows: (0..height).map(|y| (0, width, y * width)).collect(),
            len: width * height,
            tracked: false,
        }
    }

    /// The union of surface rectangles `(x0, y0, x1, y1)`, clamped to the
    /// surface. Rectangles sharing a row widen that row's span.
    fn from_rects(
        rects: impl IntoIterator<Item = (isize, isize, isize, isize)>,
        width: usize,
        height: usize,
    ) -> Self {
        let mut spans: Vec<(usize, usize, usize)> = Vec::new();
        let mut top = usize::MAX;
        let mut bottom = 0;
        let mut clamped = Vec::new();
        for (x0, y0, x1, y1) in rects {
            let x0 = x0.clamp(0, width as isize) as usize;
            let x1 = x1.clamp(0, width as isize) as usize;
            let y0 = y0.clamp(0, height as isize) as usize;
            let y1 = y1.clamp(0, height as isize) as usize;
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            top = top.min(y0);
            bottom = bottom.max(y1);
            clamped.push((x0, y0, x1, y1));
        }
        if clamped.is_empty() {
            return Self::empty();
        }
        spans.resize(bottom - top, (usize::MAX, 0, 0));
        for (x0, y0, x1, y1) in clamped {
            for span in &mut spans[y0 - top..y1 - top] {
                span.0 = span.0.min(x0);
                span.1 = span.1.max(x1);
            }
        }
        let mut len = 0;
        let rows = spans
            .into_iter()
            .map(|(x0, x1, _)| {
                let width = x1.saturating_sub(x0);
                let row = (if width == 0 { 0 } else { x0 }, width, len);
                len += width;
                row
            })
            .collect();
        Self {
            top,
            rows,
            len,
            tracked: false,
        }
    }

    /// `(first column, column count, compact offset)` of surface row `y`.
    fn row(&self, y: isize) -> Option<(usize, usize, usize)> {
        let index = usize::try_from(y).ok()?.checked_sub(self.top)?;
        self.rows.get(index).copied().filter(|row| row.1 != 0)
    }

    /// Nonempty rows `(y, x0, x1, offset)` with `y` in `y0..y1`.
    fn rows_in(
        &self,
        y0: usize,
        y1: usize,
    ) -> impl Iterator<Item = (usize, usize, usize, usize)> + '_ {
        let start = y0.saturating_sub(self.top).min(self.rows.len());
        let end = y1.saturating_sub(self.top).min(self.rows.len()).max(start);
        self.rows[start..end]
            .iter()
            .enumerate()
            .filter(|(_, row)| row.1 != 0)
            .map(move |(index, &(x0, width, offset))| {
                (self.top + start + index, x0, x0 + width, offset)
            })
    }

    /// Whether any region pixel lies in surface rectangle `(x0, y0, x1, y1)`.
    fn intersects(&self, (x0, y0, x1, y1): (usize, usize, usize, usize)) -> bool {
        self.rows_in(y0, y1)
            .any(|(_, row_x0, row_x1, _)| x0.max(row_x0) < x1.min(row_x1))
    }

    /// Call `visit` with the compact index of every region pixel in surface
    /// rectangle `(x0, y0, x1, y1)`, row by row.
    fn for_each_index_in(
        &self,
        (x0, y0, x1, y1): (usize, usize, usize, usize),
        mut visit: impl FnMut(usize),
    ) {
        for (_, row_x0, row_x1, offset) in self.rows_in(y0, y1) {
            let (start, end) = (x0.max(row_x0), x1.min(row_x1));
            if start < end {
                (offset + start - row_x0..offset + end - row_x0).for_each(&mut visit);
            }
        }
    }

    fn contains(&self, x: usize, y: usize) -> bool {
        self.row(y as isize)
            .is_some_and(|(x0, width, _)| (x0..x0 + width).contains(&x))
    }
}

/// Rows per paint-bounds bucket. A layer's painted area is kept as one
/// rectangle per bucket, so a long diagonal edge keeps a thin region.
const PAINT_BUCKET_ROWS: usize = 256;

/// Painted bounds of one effect layer: a rectangle per row bucket.
struct PaintTracker {
    pointer: usize,
    buckets: Vec<Option<(usize, usize, usize, usize)>>,
}

impl PaintTracker {
    fn add(&mut self, (x0, y0, x1, y1): (usize, usize, usize, usize)) {
        let bucket = y0 / PAINT_BUCKET_ROWS;
        if bucket >= self.buckets.len() {
            self.buckets.resize(bucket + 1, None);
        }
        let slot = &mut self.buckets[bucket];
        *slot = Some(match *slot {
            Some((l, t, r, b)) => (l.min(x0), t.min(y0), r.max(x1), b.max(y1)),
            None => (x0, y0, x1, y1),
        });
    }
}

thread_local! {
    // Effect layers whose painted pixels are being bounded, innermost last.
    static PAINT_TRACKERS: std::cell::RefCell<Vec<PaintTracker>> =
        const { std::cell::RefCell::new(Vec::new()) };
    // Compact effect destinations retain surface coordinates. Only painters
    // targeting this exact buffer inherit its storage layout; mask coverage
    // scratch has its own explicit output region.
    static LAYER_OUTPUTS: std::cell::RefCell<Vec<(usize, std::rc::Rc<LayerRegion>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn layer_output(pixels: &[u8]) -> Option<std::rc::Rc<LayerRegion>> {
    LAYER_OUTPUTS.with(|outputs| {
        outputs.borrow().last().and_then(|(pointer, region)| {
            (*pointer == pixels.as_ptr() as usize).then(|| region.clone())
        })
    })
}

struct LayerOutputScope;

impl LayerOutputScope {
    fn new(pixels: &[u8], region: std::rc::Rc<LayerRegion>) -> Self {
        LAYER_OUTPUTS.with(|outputs| {
            outputs
                .borrow_mut()
                .push((pixels.as_ptr() as usize, region));
        });
        Self
    }
}

impl Drop for LayerOutputScope {
    fn drop(&mut self) {
        LAYER_OUTPUTS.with(|outputs| outputs.borrow_mut().pop());
    }
}

/// effects.go:renderEffectNode allocates only effectRenderBounds within the
/// destination band. Simple path masks need no full-frame backing storage;
/// their rasterizers still use the original frame and reference origins.
fn masked_path_region(
    group: &usvg::Group,
    width: usize,
    height: usize,
) -> Option<std::rc::Rc<LayerRegion>> {
    if go_band_recording()
        || !group.children().iter().all(|child| {
            matches!(child, usvg::Node::Path(path)
                if path.fill().is_none_or(|fill| fill.rule() == FillRule::NonZero))
        })
    {
        return None;
    }
    let (top, bottom) = frame_band_rows(width, height)?;
    let windows = go_band_windows(width, height)?;
    Some(std::rc::Rc::new(LayerRegion::from_rects(
        windows
            .into_iter()
            .map(|(x0, y0, x1, y1)| (x0, y0.max(top as isize), x1, y1.min(bottom as isize))),
        width,
        height,
    )))
}

/// Rows one rasterizer paint writes into the innermost tracked layer.
struct PaintBounds {
    rects: Vec<(usize, usize, usize, usize)>,
}

impl PaintBounds {
    fn tracking(pixels: &[u8]) -> Option<Self> {
        let pointer = pixels.as_ptr() as usize;
        PAINT_TRACKERS
            .with(|trackers| {
                trackers
                    .borrow()
                    .last()
                    .is_some_and(|tracker| tracker.pointer == pointer)
            })
            .then(|| Self { rects: Vec::new() })
    }

    fn add_row(&mut self, y: usize, x0: usize, x1: usize) {
        if let Some(last) = self.rects.last_mut()
            && last.1 / PAINT_BUCKET_ROWS == y / PAINT_BUCKET_ROWS
        {
            *last = (
                last.0.min(x0),
                last.1.min(y),
                last.2.max(x1),
                last.3.max(y + 1),
            );
            return;
        }
        self.rects.push((x0, y, x1, y + 1));
    }

    fn commit(self) {
        PAINT_TRACKERS.with(|trackers| {
            if let Some(tracker) = trackers.borrow_mut().last_mut() {
                for rect in self.rects {
                    tracker.add(rect);
                }
            }
        });
    }
}

/// Record that `region`'s pixels of `pixels` were composited, when `pixels`
/// is the innermost tracked layer.
fn note_painted(pixels: &[u8], region: &LayerRegion) {
    let Some(mut bounds) = PaintBounds::tracking(pixels) else {
        return;
    };
    for (y, x0, x1, _) in region.rows_in(0, usize::MAX) {
        bounds.add_row(y, x0, x1);
    }
    bounds.commit();
}

/// Bounds what a new effect layer for `group` receives while its content is
/// drawn, so masks, clips and compositing visit only those pixels.
///
/// Paths and outline text paint only through `Rasterizer::paint_window`,
/// which reports the rows and columns it writes, and nested effect layers
/// report what they composite (`note_painted`). Images, color glyphs,
/// even-odd fills, filters and appendix shadows write through other paths;
/// a subtree containing them keeps the whole surface. A band record pass
/// paints nothing, so its layers are empty.
struct LayerPaintScope {
    tracking: bool,
}

impl LayerPaintScope {
    fn begin(tree: &Tree, group: &usvg::Group, layer: &[u8]) -> Self {
        Self::begin_with(paints_through_rasterizers(tree, group), layer)
    }

    /// For a layer that receives only `group`'s children.
    fn begin_contents(tree: &Tree, group: &usvg::Group, layer: &[u8]) -> Self {
        Self::begin_with(contents_paint_through_rasterizers(tree, group), layer)
    }

    fn begin_with(trackable: bool, layer: &[u8]) -> Self {
        let tracking = !go_band_recording() && trackable;
        if tracking {
            PAINT_TRACKERS.with(|trackers| {
                trackers.borrow_mut().push(PaintTracker {
                    pointer: layer.as_ptr() as usize,
                    buckets: Vec::new(),
                })
            });
        }
        Self { tracking }
    }

    fn finish(mut self, width: usize, height: usize) -> std::rc::Rc<LayerRegion> {
        if go_band_recording() {
            return std::rc::Rc::new(LayerRegion::empty());
        }
        if !self.tracking {
            return std::rc::Rc::new(surface_region(width, height));
        }
        self.tracking = false;
        let tracker = PAINT_TRACKERS
            .with(|trackers| trackers.borrow_mut().pop())
            .expect("tracked layer scope");
        let mut region = LayerRegion::from_rects(
            tracker
                .buckets
                .into_iter()
                .flatten()
                .map(|(x0, y0, x1, y1)| (x0 as isize, y0 as isize, x1 as isize, y1 as isize)),
            width,
            height,
        );
        region.tracked = true;
        std::rc::Rc::new(region)
    }
}

thread_local! {
    // While a PNG is rendered one band of rows at a time into a full-frame
    // buffer: (frame width, frame height, first row, end row).
    static FRAME_BAND: std::cell::Cell<Option<(usize, usize, usize, usize)>> =
        const { std::cell::Cell::new(None) };
}

/// Rows of a `width` x `height` surface the current band pass produces, when
/// that surface is the banded frame (or one of its full-frame layers).
pub(crate) fn frame_band_rows(width: usize, height: usize) -> Option<(usize, usize)> {
    FRAME_BAND
        .with(std::cell::Cell::get)
        .filter(|&(frame_width, frame_height, _, _)| (frame_width, frame_height) == (width, height))
        .map(|(_, _, top, bottom)| (top, bottom))
}

/// Temporarily changes the rows being painted. Filters read source pixels
/// beyond the destination band, bounded by their required input region.
struct FrameBandScope(Option<(usize, usize, usize, usize)>);

impl FrameBandScope {
    fn new(rows: Option<(usize, usize, usize, usize)>) -> Self {
        Self(FRAME_BAND.with(|band| band.replace(rows)))
    }

    /// The suspended band pass's rows, for a `width` x `height` surface.
    fn band_rows(&self, width: usize, height: usize) -> Option<(usize, usize)> {
        self.0
            .filter(|&(frame_width, frame_height, _, _)| {
                (frame_width, frame_height) == (width, height)
            })
            .map(|(_, _, top, bottom)| (top, bottom))
    }
}

impl Drop for FrameBandScope {
    fn drop(&mut self) {
        FRAME_BAND.with(|band| band.set(self.0));
    }
}

/// Every pixel of a layer whose paint is not tracked, limited to the rows of
/// the current band pass. Rows outside the band are never read back: each
/// band pass repaints its own rows of the frame from the background.
fn surface_region(width: usize, height: usize) -> LayerRegion {
    match frame_band_rows(width, height) {
        Some((top, bottom)) => LayerRegion::from_rects(
            [(0, top as isize, width as isize, bottom as isize)],
            width,
            height,
        ),
        None => LayerRegion::full(width, height),
    }
}

impl Drop for LayerPaintScope {
    fn drop(&mut self) {
        if self.tracking {
            PAINT_TRACKERS.with(|trackers| trackers.borrow_mut().pop());
        }
    }
}

fn paints_through_rasterizers(tree: &Tree, group: &usvg::Group) -> bool {
    group.filters().is_empty()
        && !group.id().starts_with("d2-rs-static-appendix")
        && contents_paint_through_rasterizers(tree, group)
}

/// `paints_through_rasterizers` for a group's children alone, as drawn by
/// draw_group_contents.
fn contents_paint_through_rasterizers(tree: &Tree, group: &usvg::Group) -> bool {
    group.children().iter().all(|child| match child {
        usvg::Node::Group(group) => paints_through_rasterizers(tree, group),
        // Even-odd fills sample their own bounds (paint_even_odd_paths).
        usvg::Node::Path(path) => path
            .fill()
            .is_none_or(|fill| fill.rule() != FillRule::EvenOdd),
        usvg::Node::Text(text) => !text_contains_color_glyph(tree, text),
        usvg::Node::Image(_) => false,
    })
}

/// One leaf primitive's contribution to effectRenderBounds, in absolute
/// surface pixels: prepared paint bounds, or a stroke-only primitive's run
/// segments with strokeRegionExtent + 1 (stroke_regions.go).
#[derive(Clone)]
enum GoLeafBounds {
    Rect(f64, f64, f64, f64),
    Stroke {
        segments: std::rc::Rc<Vec<(Point64, Point64)>>,
        expansion: f64,
        full_expansion: f64,
    },
}

/// A recorded leaf and the intersection of every enclosing clip/mask bound
/// it lies under (prepared node bounds are intersected at each level).
type GoLeaf = (GoLeafBounds, Option<(isize, isize, isize, isize)>);

thread_local! {
    // A stack of leaf lists, one per effect group open in the record pass.
    static GO_BAND_RECORD: std::cell::RefCell<Vec<Vec<GoLeaf>>> =
        const { std::cell::RefCell::new(Vec::new()) };
    // Frame-scoped prepared geometry, shared by every destination band.
    // Source bands.go prepares complete node bounds before drawing any band.
    static GO_BAND_LEAF_CACHE: std::cell::RefCell<HashMap<*const usvg::Group, std::rc::Rc<Vec<GoLeaf>>>> =
        std::cell::RefCell::new(HashMap::new());
    // A group's prepared destinations depend only on its immutable geometry
    // and the enclosing prepared effect, never on the currently emitted rows.
    static GO_BAND_LAYER_CACHE: std::cell::RefCell<HashMap<(*const usvg::Group, usize), std::rc::Rc<GoLayer>>> =
        std::cell::RefCell::new(HashMap::new());
    static GO_BAND_STROKE_ONLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn go_band_recording() -> bool {
    GO_BAND_RECORD.with(|record| !record.borrow().is_empty())
}

/// Marks the path being painted as fill-free and stroked (effectRenderBounds'
/// strokeRegionBounds case) until dropped.
struct GoStrokeOnlyScope(bool);

impl GoStrokeOnlyScope {
    fn new(stroke_only: bool) -> Self {
        Self(GO_BAND_STROKE_ONLY.with(|flag| flag.replace(stroke_only)))
    }
}

impl Drop for GoStrokeOnlyScope {
    fn drop(&mut self) {
        GO_BAND_STROKE_ONLY.with(|flag| flag.set(self.0));
    }
}

fn go_rect_intersect(
    a: (isize, isize, isize, isize),
    b: (isize, isize, isize, isize),
) -> Option<(isize, isize, isize, isize)> {
    let r = (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3));
    (r.0 < r.2 && r.1 < r.3).then_some(r)
}

fn go_rect_union(
    a: Option<(isize, isize, isize, isize)>,
    b: Option<(isize, isize, isize, isize)>,
) -> Option<(isize, isize, isize, isize)> {
    match (a, b) {
        (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))),
        (a, None) => a,
        (None, b) => b,
    }
}

/// image.Rect(floor(min), floor(min), ceil(max), ceil(max)) ∩ canvas.
fn go_float_rect(
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
    canvas: (isize, isize, isize, isize),
) -> Option<(isize, isize, isize, isize)> {
    if !(left.is_finite() && top.is_finite() && right.is_finite() && bottom.is_finite()) {
        return None;
    }
    go_rect_intersect(
        (
            left.floor() as isize,
            top.floor() as isize,
            right.ceil() as isize,
            bottom.ceil() as isize,
        ),
        canvas,
    )
}

/// Source stroke_regions.go strokeRegionBounds over recorded run segments.
fn go_stroke_region_bounds(
    segments: &[(Point64, Point64)],
    expansion: f64,
    destination: (isize, isize, isize, isize),
) -> Option<(isize, isize, isize, isize)> {
    let lower_y = destination.1 as f64 - expansion;
    let upper_y = destination.3 as f64 + expansion;
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (from, to) in segments {
        if from.y.max(to.y) < lower_y || from.y.min(to.y) > upper_y {
            continue;
        }
        let (mut first, mut last) = (0.0_f64, 1.0_f64);
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        if dy != 0.0 {
            let t0 = (lower_y - from.y) / dy;
            let t1 = (upper_y - from.y) / dy;
            first = 0.0_f64.max(t0.min(t1));
            last = 1.0_f64.min(t0.max(t1));
        }
        let (x0, x1) = (from.x + first * dx, from.x + last * dx);
        let (y0, y1) = (from.y + first * dy, from.y + last * dy);
        min_x = min_x.min(x0).min(x1);
        min_y = min_y.min(y0).min(y1);
        max_x = max_x.max(x0).max(x1);
        max_y = max_y.max(y0).max(y1);
    }
    if min_x.is_infinite() {
        return None;
    }
    let left = (min_x - expansion).max(destination.0 as f64);
    let top = (min_y - expansion).max(destination.1 as f64);
    let right = (max_x + expansion).min(destination.2 as f64);
    let bottom = (max_y + expansion).min(destination.3 as f64);
    if left >= right || top >= bottom {
        return None;
    }
    go_float_rect(left, top, right, bottom, destination)
}

/// Source effects.go effectRenderBounds for one band, from an effect node's
/// recorded leaves. `destination` is the band canvas or the parent layer.
fn go_effect_window(
    leaves: &[GoLeaf],
    canvas: (isize, isize, isize, isize),
    destination: (isize, isize, isize, isize),
) -> Option<(isize, isize, isize, isize)> {
    let visible = go_rect_intersect(go_node_bounds(leaves, canvas)?, destination)?;
    let mut bounds = None;
    for (leaf, clip) in leaves {
        let destination = match clip {
            Some(clip) => go_rect_intersect(visible, *clip),
            None => Some(visible),
        };
        let Some(destination) = destination else {
            continue;
        };
        let leaf_bounds = match leaf {
            GoLeafBounds::Rect(l, t, r, b) => go_float_rect(*l, *t, *r, *b, canvas)
                .and_then(|r| go_rect_intersect(r, destination)),
            GoLeafBounds::Stroke {
                segments,
                expansion,
                ..
            } => go_stroke_region_bounds(segments, *expansion, destination),
        };
        bounds = go_rect_union(bounds, leaf_bounds);
    }
    go_rect_intersect(bounds?, visible)
}

/// node.bounds: the union of every prepared primitive's paint bounds.
fn go_node_bounds(
    leaves: &[GoLeaf],
    canvas: (isize, isize, isize, isize),
) -> Option<(isize, isize, isize, isize)> {
    let mut node_bounds = None;
    for (leaf, clip) in leaves {
        let bounds = match leaf {
            GoLeafBounds::Rect(l, t, r, b) => go_float_rect(*l, *t, *r, *b, canvas),
            GoLeafBounds::Stroke {
                segments,
                full_expansion,
                ..
            } => {
                let (mut l, mut t) = (f64::INFINITY, f64::INFINITY);
                let (mut r, mut b) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
                for (from, to) in segments.iter() {
                    l = l.min(from.x).min(to.x);
                    t = t.min(from.y).min(to.y);
                    r = r.max(from.x).max(to.x);
                    b = b.max(from.y).max(to.y);
                }
                go_float_rect(
                    l - full_expansion,
                    t - full_expansion,
                    r + full_expansion,
                    b + full_expansion,
                    canvas,
                )
            }
        };
        let bounds = match clip {
            Some(clip) => bounds.and_then(|bounds| go_rect_intersect(bounds, *clip)),
            None => bounds,
        };
        node_bounds = go_rect_union(node_bounds, bounds);
    }
    node_bounds
}

/// The pixel bounds a group's clip and mask impose on its prepared bounds
/// (raster.go node preparation: bounds ∩ clip.bounds ∩ mask.root.bounds).
fn go_group_clip_rect(
    group: &usvg::Group,
    scale: f32,
    sources: &SvgImagePathSources,
    canvas_size: (usize, usize),
) -> Option<(isize, isize, isize, isize)> {
    let s = f64::from(scale);
    let to_rect = |bounds: usvg::Rect| -> Option<(isize, isize, isize, isize)> {
        let canvas = (
            isize::MIN / 4,
            isize::MIN / 4,
            isize::MAX / 4,
            isize::MAX / 4,
        );
        go_float_rect(
            f64::from(bounds.left()) * s - 1.0,
            f64::from(bounds.top()) * s - 1.0,
            f64::from(bounds.right()) * s + 1.0,
            f64::from(bounds.bottom()) * s + 1.0,
            canvas,
        )
    };
    // Clip and mask content lives in the referencing group's user space.
    // usvg leaves clip roots without computed bounds; use their paths.
    let clip = group
        .clip_path()
        .filter(|clip| !is_trivial_clip(clip, group))
        .and_then(|clip| {
            let base = group.abs_transform().pre_concat(clip.transform());
            clip.root()
                .children()
                .iter()
                .fold(None, |bounds: Option<usvg::Rect>, child| {
                    let usvg::Node::Path(path) = child else {
                        return bounds;
                    };
                    let Some(rect) = path
                        .data()
                        .bounds()
                        .transform(base.pre_concat(path.abs_transform()))
                    else {
                        return bounds;
                    };
                    Some(match bounds {
                        Some(bounds) => usvg::Rect::from_ltrb(
                            bounds.left().min(rect.left()),
                            bounds.top().min(rect.top()),
                            bounds.right().max(rect.right()),
                            bounds.bottom().max(rect.bottom()),
                        )
                        .unwrap_or(bounds),
                        None => rect,
                    })
                })
        })
        .and_then(to_rect);
    let mask = group.mask().and_then(|mask| {
        if let Some(native) = sources.native_connection_mask(mask.id()) {
            // d2raster.prepare.node prepares the mask in the referencing
            // node's transformed user space, before intersecting its bounds.
            // The SVG wrapper carries that local transform on its child.
            let local = sources.native_mask_transform(group.id());
            let transform = source_affine_mul(
                native_canvas_transform(native.viewport, canvas_size.0, canvas_size.1),
                Affine64 {
                    sx: local[0],
                    ky: local[1],
                    kx: local[2],
                    sy: local[3],
                    tx: local[4],
                    ty: local[5],
                },
            );
            let mut bounds = None;
            for [x, y, width, height] in &native.boxes {
                let points = [
                    Point64 { x: *x, y: *y },
                    Point64 {
                        x: x + width,
                        y: *y,
                    },
                    Point64 {
                        x: *x,
                        y: y + height,
                    },
                    Point64 {
                        x: x + width,
                        y: y + height,
                    },
                ]
                .map(|point| map_point(point, transform));
                let left = points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
                let top = points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
                let right = points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
                let bottom = points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
                bounds = go_rect_union(
                    bounds,
                    go_float_rect(
                        left - 1.0,
                        top - 1.0,
                        right + 1.0,
                        bottom + 1.0,
                        (
                            isize::MIN / 4,
                            isize::MIN / 4,
                            isize::MAX / 4,
                            isize::MAX / 4,
                        ),
                    ),
                );
            }
            bounds
        } else {
            mask.root()
                .bounding_box()
                .transform(group.abs_transform())
                .and_then(to_rect)
        }
    });
    #[cfg(feature = "diagnostic-traces")]
    if raster_trace_pixel().is_some() {
        eprintln!(
            "d2-rs-go-clip id={:?} clip={clip:?} mask={mask:?} clip_root_bbox={:?} clip_transform={:?} abs={:?}",
            group.id(),
            group.clip_path().map(|c| c.root().bounding_box()),
            group.clip_path().map(|c| c.transform()),
            group.abs_transform()
        );
    }
    match (clip, mask) {
        (Some(a), Some(b)) => go_rect_intersect(a, b).or(Some((0, 0, 0, 0))),
        (a, None) => a,
        (None, b) => b,
    }
}

fn go_clip_leaves(leaves: &mut [GoLeaf], clip: Option<(isize, isize, isize, isize)>) {
    let Some(clip) = clip else {
        return;
    };
    for (_, leaf_clip) in leaves {
        *leaf_clip = Some(match leaf_clip {
            Some(existing) => go_rect_intersect(*existing, clip).unwrap_or((0, 0, 0, 0)),
            None => clip,
        });
    }
}

struct Rasterizer {
    width: usize,
    height: usize,
    // Destination pixel buffer size. `width`/`height` are the raster window,
    // which Go band emulation narrows to one source destination at a time.
    surface_width: usize,
    surface_height: usize,
    // A destination holding only these surface pixels (set_output_region);
    // coverage is still accumulated over the whole raster window.
    output: Option<std::rc::Rc<LayerRegion>>,
    stroke_segments: Vec<(Point64, Point64)>,
    stroke_expansion: f64,
    stroke_full_expansion: f64,
    // Set when Go band emulation fixed this rasterizer's reference origin.
    go_reference_locked: bool,
    edges: Vec<Edge>,
    prepared: Option<std::rc::Rc<PreparedRaster>>,
    scan_edges: Vec<ScanEdge>,
    row_heads: Vec<usize>,
    active: Vec<usize>,
    reference_origin: Point64,
    rounding_origin: Point64,
    #[cfg(feature = "diagnostic-traces")]
    trace_origin_x: usize,
    #[cfg(feature = "diagnostic-traces")]
    trace_origin_y: usize,
    // Scan accumulation rows, sized for the constructed raster window and
    // taken from SCAN_BUFFERS when the first paint needs them.
    partial: Vec<f32>,
    difference: Vec<f32>,
    scan_width: usize,
    scan_height: usize,
    touched_min: usize,
    touched_max: usize,
    touched: bool,
    tolerance: f64,
}

/// Immutable path submission, prepared once per frame as in d2raster.prepare.
/// Band rasterizers borrow these edges and keep only scan conversion mutable.
struct PreparedRaster {
    edges: Vec<Edge>,
    // Conservative y ranges for consecutive submission-order blocks. A band
    // can reject an entire block without revisiting every off-band edge.
    edge_blocks: Vec<(f64, f64)>,
    stroke_segments: std::rc::Rc<Vec<(Point64, Point64)>>,
    stroke_expansion: f64,
    stroke_full_expansion: f64,
    reference_origin: Point64,
    rounding_origin: Point64,
    bounds: Option<(usize, usize, usize, usize)>,
}

const PREPARED_EDGE_BLOCK: usize = 32;

type PreparedRasterKey = (usize, u8, [u64; 7], [u64; 4], (usize, usize));

type PathLinesKey = (usize, [u64; 7], [u32; 6]);
struct PreparedPathLines {
    centerline: Option<(Vec<Point64>, bool)>,
    centerlines: Option<Vec<(Vec<Point64>, bool)>>,
}

type ScanBuffers = (Vec<usize>, Vec<f32>, Vec<f32>);

thread_local! {
    // Accumulation rows of finished rasterizers, for reuse. They are all
    // zero: a paint consumes (and clears) every row head it set, and clears
    // each row's touched partial/difference cells before the next row.
    static SCAN_BUFFERS: std::cell::RefCell<Vec<ScanBuffers>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

impl Drop for Rasterizer {
    fn drop(&mut self) {
        if self.partial.is_empty() {
            return;
        }
        debug_assert!(
            self.row_heads.iter().all(|&head| head == 0)
                && self
                    .partial
                    .iter()
                    .chain(&self.difference)
                    .all(|&v| v == 0.0),
            "rasterizer accumulation rows must be clear after painting"
        );
        let buffers = (
            std::mem::take(&mut self.row_heads),
            std::mem::take(&mut self.partial),
            std::mem::take(&mut self.difference),
        );
        SCAN_BUFFERS.with(|pool| {
            let mut pool = pool.borrow_mut();
            if pool.len() < 8 {
                pool.push(buffers);
            }
        });
    }
}

#[derive(Clone, Default)]
struct RasterSourceMap<'a> {
    // Source bands.go separates actual destination origin from the stable
    // full-render reference origin used for float32 path submission.
    device_origin: Point64,
    banded: bool,
    canvas_height: usize,
    band_scratch_bytes: usize,
    paths: HashMap<&'a str, &'a d2_rs_render::RasterPathSource>,
    exact_dash_parameters: HashMap<String, (f64, f64, f64)>,
    pattern_tiles: PatternInstances,
    paint_reference_contexts: HashMap<String, PaintReferenceContext>,
    native_path_transforms: HashMap<*const usvg::Path, Affine64>,
    native_text_transforms: HashMap<*const usvg::Text, Affine64>,
    // Retained effect transforms for native shadows and Markdown viewports.
    native_shadow_transforms: HashMap<*const usvg::Group, Affine64>,
    native_rects: HashMap<String, NativeRect>,
    gradient_stops: HashMap<String, crate::svg_precision::RawGradient>,
    native_gradient_paints: HashMap<String, HashMap<String, crate::svg_precision::RawGradient>>,
}

impl<'a> RasterSourceMap<'a> {
    /// Complete-frame pixel size that native scene viewports map onto. A band
    /// buffer spans the full width but only `height` of `canvas_height` rows;
    /// its device origin then addresses the band within that frame.
    fn native_canvas_size(&self, width: usize, height: usize) -> (usize, usize) {
        (
            width,
            if self.banded {
                self.canvas_height
            } else {
                height
            },
        )
    }

    fn from_paths(
        sources: &'a [d2_rs_render::RasterPathSource],
        exact_dash_parameters: &BTreeMap<String, (f64, f64, f64)>,
    ) -> Self {
        Self {
            paths: sources
                .iter()
                .map(|source| (source.id.as_str(), source))
                .collect(),
            exact_dash_parameters: exact_dash_parameters.clone().into_iter().collect(),
            pattern_tiles: HashMap::new(),
            paint_reference_contexts: HashMap::new(),
            native_path_transforms: HashMap::new(),
            native_text_transforms: HashMap::new(),
            native_shadow_transforms: HashMap::new(),
            native_rects: HashMap::new(),
            gradient_stops: HashMap::new(),
            native_gradient_paints: HashMap::new(),
            device_origin: Point64::default(),
            banded: false,
            canvas_height: 0,
            band_scratch_bytes: 0,
        }
    }

    fn get(&self, id: &str) -> Option<&&'a d2_rs_render::RasterPathSource> {
        self.paths.get(id)
    }

    fn contains_key(&self, id: &str) -> bool {
        self.paths.contains_key(id)
    }
}

// Go bands.go:setBandReferenceBounds transports the complete paint rectangle,
// not each ordinary group's ink bounds. Keep this independent of path/stroke
// integration origins, which may include a mask-layer clearance adjustment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TextPaintBounds {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

impl TextPaintBounds {
    fn intersect(self, other: Self) -> Option<Self> {
        let result = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (result.left < result.right && result.top < result.bottom).then_some(result)
    }
    fn origin(self) -> Point64 {
        Point64 {
            x: self.left as f64,
            y: self.top as f64,
        }
    }
}

// d2raster.subpathPixelBounds and setBandReferenceBounds: use the actual f64
// prepared contours, conservative stroke extent and one antialiasing pixel.
// SVG's glyph/layer boxes are not this source contract.
fn markdown_paint_bounds(
    tree: &Tree,
    group: &usvg::Group,
    viewport: [f64; 4],
    scale: f32,
    canvas: TextPaintBounds,
    sources: &SvgImagePathSources,
) -> Option<TextPaintBounds> {
    native_paint_bounds(tree, group, Some(viewport), scale, canvas, sources)
}

fn native_paint_bounds(
    tree: &Tree,
    group: &usvg::Group,
    viewport: Option<[f64; 4]>,
    scale: f32,
    canvas: TextPaintBounds,
    sources: &SvgImagePathSources,
) -> Option<TextPaintBounds> {
    native_prepared_bounds(tree, group, viewport, scale, sources)?.intersect(canvas)
}

fn native_prepared_bounds(
    tree: &Tree,
    group: &usvg::Group,
    viewport: Option<[f64; 4]>,
    scale: f32,
    sources: &SvgImagePathSources,
) -> Option<TextPaintBounds> {
    fn include(
        bounds: &mut Option<TextPaintBounds>,
        points: &[Point64],
        transform: Affine64,
        expansion: f64,
    ) {
        if points.is_empty() {
            return;
        }
        let mut raw = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for point in points {
            let point = map_point(*point, transform);
            raw[0] = raw[0].min(point.x);
            raw[1] = raw[1].min(point.y);
            raw[2] = raw[2].max(point.x);
            raw[3] = raw[3].max(point.y);
        }
        if raw.iter().any(|v| !v.is_finite()) {
            return;
        }
        let padding = expansion + 1.0;
        let b = TextPaintBounds {
            left: (raw[0] - padding).floor() as i64,
            top: (raw[1] - padding).floor() as i64,
            right: (raw[2] + padding).ceil() as i64,
            bottom: (raw[3] + padding).ceil() as i64,
        };
        if b.left < b.right && b.top < b.bottom {
            *bounds = Some(bounds.map_or(b, |old| TextPaintBounds {
                left: old.left.min(b.left),
                top: old.top.min(b.top),
                right: old.right.max(b.right),
                bottom: old.bottom.max(b.bottom),
            }));
        }
    }
    fn extent(stroke: &usvg::Stroke, transform: Affine64, width: f64) -> f64 {
        let mut factor = if stroke.linecap() == usvg::LineCap::Square {
            std::f64::consts::SQRT_2
        } else {
            1.0
        };
        if matches!(
            stroke.linejoin(),
            usvg::LineJoin::Miter | usvg::LineJoin::MiterClip
        ) {
            factor = factor.max(f64::from(stroke.miterlimit().get()));
        }
        width * affine_max_scale(transform) / 2.0 * factor
    }
    fn visit(
        tree: &Tree,
        group: &usvg::Group,
        scale: f32,
        sources: &SvgImagePathSources,
        bounds: &mut Option<TextPaintBounds>,
    ) -> Option<()> {
        for child in group.children() {
            match child {
                usvg::Node::Group(child) => {
                    if child.opacity().get() == 0.0 {
                        continue;
                    }
                    if let Some(viewport) = sources.native_markdown_viewport(child.id()) {
                        let b =
                            native_prepared_bounds(tree, child, Some(viewport), scale, sources)?;
                        *bounds = Some(bounds.map_or(b, |old| TextPaintBounds {
                            left: old.left.min(b.left),
                            top: old.top.min(b.top),
                            right: old.right.max(b.right),
                            bottom: old.bottom.max(b.bottom),
                        }));
                    } else {
                        if child.clip_path().is_some()
                            || child.mask().is_some()
                            || !child.filters().is_empty()
                            || child.isolate()
                            || child.blend_mode() != usvg::BlendMode::Normal
                        {
                            return None;
                        }
                        // Opacity changes paint bytes, not prepared geometry.
                        // Source setBandReferenceBounds still uses the full
                        // visible bounds of all qualified descendants.
                        visit(tree, child, scale, sources, bounds)?;
                    }
                }
                usvg::Node::Path(path) => {
                    let rect = *sources.native_rects().get(path.id())?;
                    let device = path.abs_transform().post_scale(scale, scale);
                    let transform = canonical_transform(device);
                    let (points, _) =
                        collect_scene_rect_centerline(rect, flatten_tolerance(device));
                    let expansion = path
                        .stroke()
                        .map_or(0.0, |s| extent(s, transform, f64::from(s.width().get())));
                    include(bounds, &points, transform, expansion);
                }
                usvg::Node::Text(text) => {
                    let origin = sources.exact_text_origin_by_id(text.id())?;
                    // Go prepares bounds from positionGlyphs' resolved faces
                    // and substitutions, just as it prepares the painted paths.
                    // usvg's missing-glyph outline can have smaller bounds.
                    let native_layout = native_mixed_font_layout_with_sources(tree, text, sources);
                    if sources.code_line(text.id()).is_some() && native_layout.is_none() {
                        return None;
                    }
                    let spans = native_layout
                        .as_deref()
                        .map_or_else(|| text.layouted(), |(spans, _)| spans.as_slice());
                    let mut placements = if let Some((_, placements)) = native_layout.as_deref() {
                        placements.clone()
                    } else {
                        fixed_glyph_transforms(tree, text, spans)?
                    };
                    apply_prepared_text_origins(&mut placements, text, origin, true, sources)?;
                    let device = text.abs_transform().post_scale(scale, scale);
                    let transform = synthetic_italic_pivot(text)
                        .and_then(|(x, y)| synthetic_italic_device_transform(device, x, y))
                        .unwrap_or_else(|| canonical_transform(device));
                    for (span_index, span) in spans.iter().enumerate() {
                        let expansion = span.stroke.as_ref().map_or(0.0, |s| {
                            extent(
                                s,
                                transform,
                                sources.text_stroke_width(text.id(), s.width().get()),
                            )
                        });
                        for (glyph_index, glyph) in span.positioned_glyphs.iter().enumerate() {
                            let placement = placements[span_index][glyph_index];
                            let Some(path) = fixed_glyph_path(tree, glyph, placement) else {
                                if glyph.text.chars().all(|value| {
                                    value.is_whitespace() || crate::font_shaping::ignorable(value)
                                }) {
                                    continue;
                                }
                                // fontface GlyphRenderBounds: a trusted COLRv1
                                // glyph is bounded by its static clip box.
                                let [x0, y0, x1, y1] = tree
                                    .fontdb()
                                    .with_face_data(glyph.font, |data, _| {
                                        crate::colr_plan::Compiler::new(data)
                                            .ok()?
                                            .compile(glyph.id.0)
                                            .ok()?
                                            .and_then(|plan| plan.clip)
                                    })
                                    .flatten()?;
                                let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                                    .map(|(x, y)| map_point(Point64 { x, y }, placement));
                                include(bounds, &corners, transform, expansion);
                                continue;
                            };
                            for (points, _) in
                                collect_fixed_subpaths(&path, flatten_tolerance(device))
                            {
                                include(bounds, &points, transform, expansion);
                            }
                        }
                        if span.underline.is_some() {
                            let size = sources.text_font_size(text.id(), span.font_size.get());
                            let descent = native_text_descent(tree, text, size)?;
                            let (left, right) =
                                fixed_decoration_bounds(tree, span, &placements[span_index])?;
                            let top = placements[span_index].first()?.ty
                                + (size / 12.0).max(descent * 0.25);
                            let thickness = (size / 16.0).max(1.0 / affine_max_scale(transform));
                            include(
                                bounds,
                                &[
                                    Point64 { x: left, y: top },
                                    Point64 { x: right, y: top },
                                    Point64 {
                                        x: right,
                                        y: top + thickness,
                                    },
                                    Point64 {
                                        x: left,
                                        y: top + thickness,
                                    },
                                ],
                                transform,
                                expansion,
                            );
                        }
                    }
                }
                _ => return None,
            }
        }
        Some(())
    }
    // Source raster.go prepares the union of complete child bounds before
    // clipping the group. Cropping individual primitives here is not
    // equivalent when disjoint children straddle a viewport boundary.
    let mut content = None;
    visit(tree, group, scale, sources, &mut content)?;
    let Some(viewport) = viewport else {
        return content;
    };
    let mut clip = None;
    let transform = canonical_transform(group.abs_transform().post_scale(scale, scale));
    include(
        &mut clip,
        &[
            Point64 {
                x: viewport[0],
                y: viewport[1],
            },
            Point64 {
                x: viewport[0] + viewport[2],
                y: viewport[1],
            },
            Point64 {
                x: viewport[0] + viewport[2],
                y: viewport[1] + viewport[3],
            },
            Point64 {
                x: viewport[0],
                y: viewport[1] + viewport[3],
            },
        ],
        transform,
        0.0,
    );
    content?.intersect(clip?)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PaintReferenceContext {
    // None marks an effect boundary whose Go prepared bounds are not yet
    // transported. usvg's f32 ink/layer bounds are not a qualified substitute.
    paint_bounds: Option<TextPaintBounds>,
}

fn collect_paint_reference_contexts(
    group: &usvg::Group,
    parent: PaintReferenceContext,
    contexts: &mut HashMap<String, PaintReferenceContext>,
    svg_image_paths: &SvgImagePathSources,
    tree: &Tree,
    scale: f32,
) {
    // usvg introduces a clip around the native renderer's serialization
    // viewport. Only that source-qualified wrapper is ordinary Go scene
    // ancestry; real clips and all other effect boundaries remain unknown.
    let native_canvas_viewport = svg_image_paths.is_native_canvas_viewport(group.id());
    let context = if let Some(viewport) = svg_image_paths.native_markdown_viewport(group.id()) {
        PaintReferenceContext {
            paint_bounds: parent.paint_bounds.and_then(|canvas| {
                markdown_paint_bounds(tree, group, viewport, scale, canvas, svg_image_paths)
            }),
        }
    } else if group.opacity().get() < 1.0
        && group.clip_path().is_none()
        && group.mask().is_none()
        && group.filters().is_empty()
        && !group.isolate()
        && group.blend_mode() == usvg::BlendMode::Normal
    {
        PaintReferenceContext {
            paint_bounds: parent.paint_bounds.and_then(|canvas| {
                native_paint_bounds(tree, group, None, scale, canvas, svg_image_paths)
            }),
        }
    } else if (group.clip_path().is_some() && !native_canvas_viewport)
        || group.mask().is_some()
        || !group.filters().is_empty()
        || group.isolate()
        || group.opacity().get() < 1.0
        || group.blend_mode() != usvg::BlendMode::Normal
    {
        PaintReferenceContext::default()
    } else {
        parent
    };
    // The retained scene crops the complete Markdown effect layer before
    // converting vector points to the scanline float32 domain. Paths and text
    // share its prepared bounds, including the inherited parent intersection.
    if svg_image_paths
        .native_markdown_viewport(group.id())
        .is_some()
    {
        contexts.insert(group.id().to_owned(), context);
    }
    for child in group.children() {
        match child {
            usvg::Node::Group(child) => {
                collect_paint_reference_contexts(
                    child,
                    context,
                    contexts,
                    svg_image_paths,
                    tree,
                    scale,
                );
            }
            usvg::Node::Text(text) if !text.id().is_empty() => {
                contexts.insert(text.id().to_owned(), context);
            }
            // Imported SVG and pattern resources start their own local canvas;
            // their text is intentionally absent from this main-scene map.
            _ => {}
        }
    }
}

fn text_reference_origin(
    legacy_origin: Point64,
    private_text_run: bool,
    context: Option<PaintReferenceContext>,
) -> Point64 {
    // Only native private TextRuns with a qualified inherited paint rectangle
    // use the retained Go reference. Untagged text and unknown effect bounds
    // preserve the existing raster path until those source bounds are carried.
    let source_origin = private_text_run
        .then_some(context)
        .flatten()
        .and_then(|context| context.paint_bounds)
        .map(TextPaintBounds::origin);
    source_origin.unwrap_or(legacy_origin)
}

impl Rasterizer {
    fn new(width: usize, height: usize, tolerance: f64) -> Self {
        let mut rasterizer = Self::new_unreferenced(width, height, tolerance);
        if let Some(reference) = go_layer_reference(width, height) {
            rasterizer.reference_origin = reference;
            rasterizer.rounding_origin = reference;
            rasterizer.go_reference_locked = true;
        }
        rasterizer
    }

    fn new_unreferenced(width: usize, height: usize, tolerance: f64) -> Self {
        Self {
            width,
            height,
            surface_width: width,
            surface_height: height,
            output: None,
            stroke_segments: Vec::new(),
            stroke_expansion: 0.0,
            stroke_full_expansion: 0.0,
            go_reference_locked: false,
            edges: Vec::new(),
            prepared: None,
            scan_edges: Vec::new(),
            row_heads: Vec::new(),
            active: Vec::new(),
            reference_origin: Point64 { x: 0.0, y: 0.0 },
            rounding_origin: Point64 { x: 0.0, y: 0.0 },
            #[cfg(feature = "diagnostic-traces")]
            trace_origin_x: 0,
            #[cfg(feature = "diagnostic-traces")]
            trace_origin_y: 0,
            partial: Vec::new(),
            difference: Vec::new(),
            scan_width: width,
            scan_height: height,
            touched_min: width,
            touched_max: 0,
            touched: false,
            tolerance,
        }
    }

    fn add_path(&mut self, path: &Path) {
        let mut current = None;
        let mut first = None;
        for segment in path.segments() {
            match segment {
                PathSegment::MoveTo(point) => {
                    current = Some(Point64 {
                        x: f64::from(point.x),
                        y: f64::from(point.y),
                    });
                    first = current;
                }
                PathSegment::LineTo(point) => {
                    let next = Point64 {
                        x: f64::from(point.x),
                        y: f64::from(point.y),
                    };
                    if let Some(from) = current {
                        self.add_edge(from, next);
                    }
                    current = Some(next);
                }
                PathSegment::QuadTo(control, point) => {
                    let Some(from) = current else { continue };
                    let to = Point64 {
                        x: f64::from(point.x),
                        y: f64::from(point.y),
                    };
                    let c = Point64 {
                        x: f64::from(control.x),
                        y: f64::from(control.y),
                    };
                    let from = self.reference_point(from);
                    let to_local = self.reference_point(to);
                    let c = self.reference_point(c);
                    // Convert a quadratic to its equivalent cubic before
                    // flattening. This is the same device-space tolerance
                    // used for cubic SVG paths.
                    let c0 = Point64 {
                        x: from.x + (c.x - from.x) * 2.0 / 3.0,
                        y: from.y + (c.y - from.y) * 2.0 / 3.0,
                    };
                    let c1 = Point64 {
                        x: to_local.x + (c.x - to_local.x) * 2.0 / 3.0,
                        y: to_local.y + (c.y - to_local.y) * 2.0 / 3.0,
                    };
                    self.add_cubic(from, c0, c1, to_local, 0);
                    current = Some(to);
                }
                PathSegment::CubicTo(c0, c1, point) => {
                    let Some(from) = current else { continue };
                    let c0 = Point64 {
                        x: f64::from(c0.x),
                        y: f64::from(c0.y),
                    };
                    let c1 = Point64 {
                        x: f64::from(c1.x),
                        y: f64::from(c1.y),
                    };
                    let to = Point64 {
                        x: f64::from(point.x),
                        y: f64::from(point.y),
                    };
                    self.add_cubic(
                        self.reference_point(from),
                        self.reference_point(c0),
                        self.reference_point(c1),
                        self.reference_point(to),
                        0,
                    );
                    current = Some(to);
                }
                PathSegment::Close => {
                    if let (Some(from), Some(to)) = (current, first) {
                        self.add_edge(from, to);
                    }
                    current = first;
                }
            }
        }
    }

    fn set_trace_origin(&mut self, origin: Point64) {
        #[cfg(feature = "diagnostic-traces")]
        {
            self.trace_origin_x = origin.x.max(0.0) as usize;
            self.trace_origin_y = origin.y.max(0.0) as usize;
        }
        #[cfg(not(feature = "diagnostic-traces"))]
        let _ = origin;
    }

    fn edges(&self) -> &[Edge] {
        self.prepared
            .as_ref()
            .map_or(&self.edges, |prepared| &prepared.edges)
    }

    fn prepare_path(
        &mut self,
        path: &usvg::Path,
        role: u8,
        transform: Affine64,
        tolerance: f64,
        prepare: impl FnOnce(&mut Self),
    ) {
        // Cropped filter buffers and explicit band buffers have changing
        // coordinate systems. Keep their scratch local, rather than retaining
        // one transformed copy per band in the complete-frame preparation.
        if go_band_recording()
            || !TEXT_MEMO.with(|memo| {
                memo.borrow()
                    .as_ref()
                    .is_some_and(|memo| memo.raster_surface == Some((self.width, self.height)))
            })
        {
            prepare(self);
            return;
        }
        let key = (
            path as *const usvg::Path as usize,
            role,
            [
                transform.sx,
                transform.kx,
                transform.ky,
                transform.sy,
                transform.tx,
                transform.ty,
                tolerance,
            ]
            .map(f64::to_bits),
            [
                self.reference_origin.x,
                self.reference_origin.y,
                self.rounding_origin.x,
                self.rounding_origin.y,
            ]
            .map(f64::to_bits),
            (self.width, self.height),
        );
        let prepared = text_memo(
            |memo| &mut memo.rasters,
            key,
            || {
                prepare(self);
                let bounds = self.fill_pixel_bounds();
                let edges = std::mem::take(&mut self.edges);
                let edge_blocks = if go_band_frame_active(self.width, self.height)
                    && edges.len() > PREPARED_EDGE_BLOCK * 2
                    // Whole-primitive bounds already reject short paths in
                    // other bands. Keep an edge index only for tall paths;
                    // retaining one for every glyph wastes frame storage.
                    && bounds.is_some_and(|(_, top, _, bottom)| {
                        bottom.saturating_sub(top) > GO_PNG_BAND_HEIGHT
                    }) {
                    edges
                        .chunks(PREPARED_EDGE_BLOCK)
                        .map(|block| {
                            block.iter().fold(
                                (f64::INFINITY, f64::NEG_INFINITY),
                                |(top, bottom), edge| {
                                    (
                                        top.min(edge.from.y).min(edge.to.y),
                                        bottom.max(edge.from.y).max(edge.to.y),
                                    )
                                },
                            )
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                std::rc::Rc::new(PreparedRaster {
                    bounds,
                    edges,
                    edge_blocks,
                    stroke_segments: std::rc::Rc::new(std::mem::take(&mut self.stroke_segments)),
                    stroke_expansion: self.stroke_expansion,
                    stroke_full_expansion: self.stroke_full_expansion,
                    reference_origin: self.reference_origin,
                    rounding_origin: self.rounding_origin,
                })
            },
        );
        self.reference_origin = prepared.reference_origin;
        self.rounding_origin = prepared.rounding_origin;
        self.prepared = Some(prepared);
    }

    /// Surface pixel rectangle `(x0, y0, x1, y1)`, end-exclusive, that a fill
    /// of the current edges can reach: their bounds plus antialiasing margin,
    /// shifted by the reference origin as `paint_window` addresses pixels.
    /// None when there is nothing to paint.
    fn fill_pixel_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        if let Some(prepared) = &self.prepared {
            return prepared.bounds;
        }
        self.pixel_bounds(self.edges.iter().flat_map(|edge| [edge.from, edge.to]))
    }

    fn pixel_bounds(
        &self,
        mut points: impl Iterator<Item = Point64>,
    ) -> Option<(usize, usize, usize, usize)> {
        let first = points.next()?;
        let (mut min, mut max) = (first, first);
        for point in points {
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
        }
        if ![min.x, min.y, max.x, max.y]
            .iter()
            .all(|value| value.is_finite())
        {
            return Some((0, 0, self.surface_width, self.surface_height));
        }
        let span = |low: f64, high: f64, reference: f64, limit: usize| {
            let reference = reference.round();
            let start = ((low + reference).floor() - 2.0).clamp(0.0, limit as f64) as usize;
            let end = ((high + reference).ceil() + 2.0).clamp(0.0, limit as f64) as usize;
            (start, end)
        };
        let (x0, x1) = span(min.x, max.x, self.reference_origin.x, self.surface_width);
        let (y0, y1) = span(min.y, max.y, self.reference_origin.y, self.surface_height);
        Some((x0, y0, x1, y1))
    }

    /// Attach zeroed accumulation rows. Rasterizers that never paint (record
    /// passes, culled primitives) allocate none.
    fn ensure_scan_buffers(&mut self) {
        if self.partial.len() == self.scan_width && !self.difference.is_empty() {
            return;
        }
        let (mut row_heads, mut partial, mut difference) = SCAN_BUFFERS
            .with(|pool| pool.borrow_mut().pop())
            .unwrap_or_default();
        row_heads.resize(self.scan_height, 0);
        partial.resize(self.scan_width, 0.0);
        difference.resize(self.scan_width + 1, 0.0);
        self.row_heads = row_heads;
        self.partial = partial;
        self.difference = difference;
    }

    /// Paint into a compact buffer of `region`'s pixels. Rows and pixels
    /// outside it are scanned exactly as before but not written.
    fn set_output_region(&mut self, region: std::rc::Rc<LayerRegion>) {
        self.output = Some(region);
    }

    fn add_fill_polyline(&mut self, points: &[Point64], _closed: bool) {
        for pair in points.windows(2) {
            self.add_edge(pair[0], pair[1]);
        }
        // SVG fill implicitly closes open subpaths before applying the
        // non-zero winding rule. This matters for line-shaped scene paths:
        // their default black fill has zero area and must not become a
        // one-sided stripe in the scan converter.
        if points.len() > 1 && points.first() != points.last() {
            self.add_edge(*points.last().unwrap(), points[0]);
        }
    }

    // Original scanline/rasterizer.go round64Point: round relative to the
    // prepared full-render origin, then apply the integer surface offset.
    fn add_fill_polyline_with_origin_offset(&mut self, points: &[Point64], offset: Point64) {
        let rounded = |p: Point64| Point64 {
            x: p.x as f32 as f64 + offset.x,
            y: p.y as f32 as f64 + offset.y,
        };
        for pair in points.windows(2) {
            self.add_edge_raw(rounded(pair[0]), rounded(pair[1]));
        }
        if points.len() > 1 && points.first() != points.last() {
            self.add_edge_raw(rounded(*points.last().unwrap()), rounded(points[0]));
        }
    }

    fn add_stroke_polyline(
        &mut self,
        points: &[Point64],
        transform: Affine64,
        width: f64,
        closed: bool,
        join: usvg::LineJoin,
        miter_limit: f64,
        cap: usvg::LineCap,
    ) {
        let cleaned_points = clean_stroke_points(points, closed);
        let points = cleaned_points.as_slice();
        if points.len() < 2 || width <= 0.0 {
            return;
        }
        let half = width * 0.5;
        if go_band_recording() {
            // geometry.go strokeExtent and stroke_regions.go
            // strokeRegionExtent, in device pixels (Matrix.MaxScale).
            let (a, b, c, d) = (transform.sx, transform.ky, transform.kx, transform.sy);
            let sum = a * a + b * b + c * c + d * d;
            let diff =
                ((a * a + b * b - c * c - d * d).powi(2) + 4.0 * (a * c + b * d).powi(2)).sqrt();
            let max_scale = ((sum + diff) / 2.0).sqrt();
            let mut factor: f64 = 1.0;
            if cap == usvg::LineCap::Square {
                factor = std::f64::consts::SQRT_2;
            }
            if join == usvg::LineJoin::Miter && miter_limit > factor {
                factor = miter_limit;
            }
            let extent = width * max_scale / 2.0 * factor;
            let mut region = extent;
            if cap == usvg::LineCap::Round || join == usvg::LineJoin::Round {
                region =
                    region.max(width / 2.0 * max_scale * 1.0_f64.hypot(0.552_284_749_830_793_6));
            }
            self.stroke_expansion = self.stroke_expansion.max(region + 1.0);
            self.stroke_full_expansion = self.stroke_full_expansion.max(extent + 1.0);
            let count = if closed {
                points.len()
            } else {
                points.len() - 1
            };
            for index in 0..count {
                self.stroke_segments.push((
                    map_point(points[index], transform),
                    map_point(points[(index + 1) % points.len()], transform),
                ));
            }
        }
        let edge_count = if closed {
            points.len()
        } else {
            points.len() - 1
        };
        for edge in 0..edge_count {
            let mut start = points[edge];
            let mut end = points[(edge + 1) % points.len()];
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            // d2raster builds each stroke rectangle with `math.Hypot`; keep
            // that operation here instead of expanding the square root so
            // the resulting vertices cross Go's float32 raster boundary with
            // identical rounding at fractional route endpoints.
            let length = d2_rs_render::go_float::gomath::hypot(dx, dy);
            if length == 0.0 {
                continue;
            }
            // Source geometry.go addStrokeRun extends the segment itself.
            // Separate overlapping cap polygons change analytic coverage at
            // their internal seams after float32 coordinate rounding.
            if !closed && cap == usvg::LineCap::Square {
                if edge == 0 {
                    start.x -= dx / length * half;
                    start.y -= dy / length * half;
                }
                if edge == edge_count - 1 {
                    end.x += dx / length * half;
                    end.y += dy / length * half;
                }
            }
            let normal = Point64 {
                x: -dy / length * half,
                y: dx / length * half,
            };
            self.add_polygon_transformed(
                &[
                    Point64 {
                        x: start.x + normal.x,
                        y: start.y + normal.y,
                    },
                    Point64 {
                        x: end.x + normal.x,
                        y: end.y + normal.y,
                    },
                    Point64 {
                        x: end.x - normal.x,
                        y: end.y - normal.y,
                    },
                    Point64 {
                        x: start.x - normal.x,
                        y: start.y - normal.y,
                    },
                ],
                transform,
            );
        }

        let join_start = if closed { 0 } else { 1 };
        let join_end = if closed {
            points.len()
        } else {
            points.len() - 1
        };
        for index in join_start..join_end {
            let previous = points[(index + points.len() - 1) % points.len()];
            let vertex = points[index];
            let next = points[(index + 1) % points.len()];
            let incoming = unit_vector(previous, vertex);
            let outgoing = unit_vector(vertex, next);
            let (Some(incoming), Some(outgoing)) = (incoming, outgoing) else {
                continue;
            };
            if join == usvg::LineJoin::Round {
                // D2 adds the round disk at every interior point, including
                // perfectly straight continuations. The disk is geometrically
                // redundant there, but its scanline edges affect byte coverage
                // at cropped, half-pixel stroke boundaries.
                self.add_round_circle_transformed(vertex, half, transform);
                continue;
            }
            let turn = incoming.x * outgoing.y - incoming.y * outgoing.x;
            if turn.abs() <= 1e-12 {
                continue;
            }
            match join {
                usvg::LineJoin::Round => unreachable!("round joins were handled above"),
                usvg::LineJoin::Bevel | usvg::LineJoin::MiterClip => self.add_join_wedge(
                    vertex,
                    incoming,
                    outgoing,
                    half,
                    false,
                    miter_limit,
                    transform,
                ),
                usvg::LineJoin::Miter => self.add_join_wedge(
                    vertex,
                    incoming,
                    outgoing,
                    half,
                    true,
                    miter_limit,
                    transform,
                ),
            }
        }
        if !closed {
            match cap {
                usvg::LineCap::Round => {
                    self.add_round_circle_transformed(points[0], half, transform);
                    self.add_round_circle_transformed(*points.last().unwrap(), half, transform);
                }
                usvg::LineCap::Square => {}
                usvg::LineCap::Butt => {}
            }
        }
    }

    fn dashed_centerline_exact(
        points: &[Point64],
        closed: bool,
        dashes: &[f32],
        dash_offset: f32,
        exact: Option<(f64, f64, f64)>,
    ) -> Vec<StrokeRun> {
        Self::dashed_centerline_impl(points, closed, dashes, dash_offset, exact, None)
    }
    fn dashed_centerline_f64(
        points: &[Point64],
        closed: bool,
        dashes: &[f64],
        offset: f64,
    ) -> Vec<StrokeRun> {
        Self::dashed_centerline_impl(points, closed, &[], 0., None, Some((dashes, offset)))
    }
    fn dashed_centerline_impl(
        points: &[Point64],
        closed: bool,
        dashes: &[f32],
        dash_offset: f32,
        exact: Option<(f64, f64, f64)>,
        native: Option<(&[f64], f64)>,
    ) -> Vec<StrokeRun> {
        if points.len() < 2 || (dashes.is_empty() && native.is_none_or(|v| v.0.is_empty())) {
            return vec![StrokeRun {
                points: points.to_vec(),
                closed,
            }];
        }
        let (mut pattern, exact_offset) = if let Some((values, offset)) = native {
            (values.to_vec(), Some(offset))
        } else if let Some((dash, gap, offset)) = exact {
            if dash.is_finite()
                && gap.is_finite()
                && offset.is_finite()
                && dash >= 0.0
                && gap >= 0.0
            {
                (vec![dash, gap], Some(offset))
            } else {
                (Vec::new(), None)
            }
        } else {
            (
                dashes
                    .iter()
                    .copied()
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .map(f64::from)
                    .collect::<Vec<_>>(),
                None,
            )
        };
        if pattern.is_empty() || pattern.iter().all(|value| *value == 0.0) {
            return Vec::new();
        }
        if pattern.len() % 2 == 1 {
            let copy = pattern.clone();
            pattern.extend(copy);
        }
        let period = pattern.iter().sum::<f64>();
        if !period.is_finite() || period <= 0.0 {
            return Vec::new();
        }

        let mut path = points.to_vec();
        if closed {
            if path.first() != path.last() {
                path.push(path[0]);
            }
        }
        let mut phase = exact_offset
            .unwrap_or_else(|| f64::from(dash_offset))
            .rem_euclid(period);
        let mut pattern_index = 0usize;
        while phase >= pattern[pattern_index] && pattern[pattern_index] > 0.0 {
            phase -= pattern[pattern_index];
            pattern_index = (pattern_index + 1) % pattern.len();
        }
        let mut remaining = (pattern[pattern_index] - phase).max(0.0);
        let mut on = pattern_index % 2 == 0;
        let initial_on = on;
        let mut runs = Vec::new();
        let mut current = Vec::new();

        for pair in path.windows(2) {
            let start = pair[0];
            let end = pair[1];
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            let length = d2_rs_render::go_float::gomath::hypot(dx, dy);
            if !length.is_finite() || length <= 0.0 {
                continue;
            }
            let mut consumed = 0.0;
            while consumed < length {
                if remaining <= f64::EPSILON {
                    pattern_index = (pattern_index + 1) % pattern.len();
                    remaining = pattern[pattern_index];
                    on = pattern_index % 2 == 0;
                    if !on && !current.is_empty() {
                        runs.push(std::mem::take(&mut current));
                    }
                    if remaining <= f64::EPSILON {
                        continue;
                    }
                }
                let step = remaining.min(length - consumed);
                let t0 = consumed / length;
                let t1 = (consumed + step) / length;
                let p0 = Point64 {
                    x: dx.mul_add(t0, start.x),
                    y: dy.mul_add(t0, start.y),
                };
                let p1 = Point64 {
                    x: dx.mul_add(t1, start.x),
                    y: dy.mul_add(t1, start.y),
                };
                if on {
                    // Source retains both interpolated endpoints. Adjacent
                    // vertices can differ by one f64 bit after interpolation;
                    // the resulting tiny segment still has a stroke normal.
                    if current.last() != Some(&p0) {
                        current.push(p0);
                    }
                    if current.last() != Some(&p1) {
                        current.push(p1);
                    }
                }
                consumed += step;
                remaining -= step;
            }
        }
        if !current.is_empty() {
            runs.push(current);
        }
        let mut runs = runs
            .into_iter()
            .map(|points| StrokeRun {
                points,
                closed: false,
            })
            .collect::<Vec<_>>();
        // Source geometry.go::makeStrokeRuns joins painted pieces across the
        // contour's closing seam. The seam is a join, not two independent caps.
        if closed && !runs.is_empty() {
            let origin = points[0];
            let starts_at_origin = runs[0].points.first() == Some(&origin);
            let ends_at_origin = runs.last().unwrap().points.last() == Some(&origin);
            if runs.len() == 1 && starts_at_origin && ends_at_origin {
                runs[0].points.pop();
                runs[0].closed = true;
            } else if initial_on && starts_at_origin && ends_at_origin {
                let mut merged = runs.pop().unwrap().points;
                merged.extend_from_slice(&runs[0].points[1..]);
                runs[0].points = merged;
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        Self::trace_dashed_centerline(points, closed, &pattern, exact_offset, dash_offset, &runs);
        runs
    }

    #[cfg(feature = "diagnostic-traces")]
    fn trace_dashed_centerline(
        points: &[Point64],
        closed: bool,
        pattern: &[f64],
        exact_offset: Option<f64>,
        fallback_offset: f32,
        runs: &[StrokeRun],
    ) {
        let Some(trace_path) = std::env::var_os("D2_RS_DASH_RUN_TRACE") else {
            return;
        };
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(trace_path)
        else {
            return;
        };
        let offset = exact_offset.unwrap_or(f64::from(fallback_offset));
        let _ = write!(
            file,
            "closed={closed} offset={:016x} pattern=",
            offset.to_bits()
        );
        for (index, dash) in pattern.iter().enumerate() {
            if index > 0 {
                let _ = write!(file, ",");
            }
            let _ = write!(file, "{:016x}", dash.to_bits());
        }
        let _ = write!(file, " points=");
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                let _ = write!(file, ";");
            }
            let _ = write!(
                file,
                "{:016x},{:016x}",
                point.x.to_bits(),
                point.y.to_bits()
            );
        }
        let _ = write!(file, " runs=");
        for (run_index, run) in runs.iter().enumerate() {
            if run_index > 0 {
                let _ = write!(file, "/");
            }
            // Retain per-run closure alongside the original diagnostic format.
            let _ = write!(file, "{}:", if run.closed { "closed" } else { "open" });
            for (point_index, point) in run.points.iter().enumerate() {
                if point_index > 0 {
                    let _ = write!(file, ";");
                }
                let _ = write!(
                    file,
                    "{:016x},{:016x}",
                    point.x.to_bits(),
                    point.y.to_bits()
                );
            }
        }
        let _ = writeln!(file);
    }

    fn add_join_wedge(
        &mut self,
        vertex: Point64,
        incoming: Point64,
        outgoing: Point64,
        half: f64,
        miter: bool,
        miter_limit: f64,
        transform: Affine64,
    ) {
        let turn = incoming.x * outgoing.y - incoming.y * outgoing.x;
        let side = if turn > 0.0 { -1.0 } else { 1.0 };
        let in_normal = Point64 {
            x: -incoming.y * half * side,
            y: incoming.x * half * side,
        };
        let out_normal = Point64 {
            x: -outgoing.y * half * side,
            y: outgoing.x * half * side,
        };
        let outer_in = Point64 {
            x: vertex.x + in_normal.x,
            y: vertex.y + in_normal.y,
        };
        let outer_out = Point64 {
            x: vertex.x + out_normal.x,
            y: vertex.y + out_normal.y,
        };
        if miter {
            let denominator = incoming.x * outgoing.y - incoming.y * outgoing.x;
            if denominator.abs() > 1e-12 {
                let delta = Point64 {
                    x: outer_out.x - outer_in.x,
                    y: outer_out.y - outer_in.y,
                };
                let distance = (delta.x * outgoing.y - delta.y * outgoing.x) / denominator;
                let miter_point = Point64 {
                    x: outer_in.x + distance * incoming.x,
                    y: outer_in.y + distance * incoming.y,
                };
                let miter_distance = ((miter_point.x - vertex.x).powi(2)
                    + (miter_point.y - vertex.y).powi(2))
                .sqrt();
                if miter_distance.is_finite() && miter_distance <= miter_limit * half {
                    self.add_polygon_transformed(
                        &[outer_in, miter_point, outer_out, vertex],
                        transform,
                    );
                    return;
                }
            }
        }
        self.add_polygon_transformed(&[outer_in, vertex, outer_out], transform);
    }

    fn add_polygon_transformed(&mut self, points: &[Point64], transform: Affine64) {
        if points.len() < 3 {
            return;
        }
        let transformed = points
            .iter()
            .copied()
            .map(|point| map_point(point, transform))
            .collect::<Vec<_>>();
        let area = signed_area(&transformed);
        if area == 0.0 || !area.is_finite() {
            return;
        }
        let want_negative = transform.sx * transform.sy - transform.kx * transform.ky >= 0.0;
        let reverse = (area < 0.0) != want_negative;
        if reverse {
            let mut iter = transformed.iter().rev();
            let first = *iter.next().unwrap();
            let mut previous = first;
            for point in iter {
                self.add_edge(previous, *point);
                previous = *point;
            }
            self.add_edge(previous, first);
        } else {
            for pair in transformed.windows(2) {
                self.add_edge(pair[0], pair[1]);
            }
            self.add_edge(*transformed.last().unwrap(), transformed[0]);
        }
    }

    fn add_round_circle_transformed(&mut self, center: Point64, radius: f64, transform: Affine64) {
        let k = 0.552_284_749_830_793_6 * radius;
        let curves = [
            (
                Point64 {
                    x: center.x + radius,
                    y: center.y,
                },
                Point64 {
                    x: center.x + radius,
                    y: center.y - k,
                },
                Point64 {
                    x: center.x + k,
                    y: center.y - radius,
                },
                Point64 {
                    x: center.x,
                    y: center.y - radius,
                },
            ),
            (
                Point64 {
                    x: center.x,
                    y: center.y - radius,
                },
                Point64 {
                    x: center.x - k,
                    y: center.y - radius,
                },
                Point64 {
                    x: center.x - radius,
                    y: center.y - k,
                },
                Point64 {
                    x: center.x - radius,
                    y: center.y,
                },
            ),
            (
                Point64 {
                    x: center.x - radius,
                    y: center.y,
                },
                Point64 {
                    x: center.x - radius,
                    y: center.y + k,
                },
                Point64 {
                    x: center.x - k,
                    y: center.y + radius,
                },
                Point64 {
                    x: center.x,
                    y: center.y + radius,
                },
            ),
            (
                Point64 {
                    x: center.x,
                    y: center.y + radius,
                },
                Point64 {
                    x: center.x + k,
                    y: center.y + radius,
                },
                Point64 {
                    x: center.x + radius,
                    y: center.y + k,
                },
                Point64 {
                    x: center.x + radius,
                    y: center.y,
                },
            ),
        ];
        let first = self.reference_point(map_point(curves[0].0, transform));
        let mut current = first;
        for (_, control0, control1, end) in curves {
            let control0 = self.reference_point(map_point(control0, transform));
            let control1 = self.reference_point(map_point(control1, transform));
            let end = self.reference_point(map_point(end, transform));
            self.add_cubic(current, control0, control1, end, 0);
            current = end;
        }
        self.add_edge_raw(current, first);
    }

    fn add_edge(&mut self, from: Point64, to: Point64) {
        // Straight path commands use D2's 64-bit API, which rounds their
        // submitted coordinates to float32 before constructing an edge.
        let from = self.reference_point(from);
        let to = self.reference_point(to);
        self.add_edge_raw(from, to);
    }

    fn add_edge_raw(&mut self, from: Point64, to: Point64) {
        if from.x == to.x && from.y == to.y || from.y == to.y {
            return;
        }
        if from.x.is_finite() && from.y.is_finite() && to.x.is_finite() && to.y.is_finite() {
            self.edges.push(Edge {
                from,
                to,
                hull_y: None,
            });
        }
    }

    fn set_reference_origin(&mut self, origin: Point64) {
        if self.go_reference_locked {
            return;
        }
        self.reference_origin = origin;
        self.rounding_origin = origin;
    }

    fn set_rounding_origin(&mut self, origin: Point64) {
        if self.go_reference_locked {
            return;
        }
        self.rounding_origin = origin;
    }

    fn reference_point(&self, point: Point64) -> Point64 {
        let rounding_offset_x = self.rounding_origin.x - self.reference_origin.x;
        let rounding_offset_y = self.rounding_origin.y - self.reference_origin.y;
        Point64 {
            x: (point.x - self.rounding_origin.x) as f32 as f64 + rounding_offset_x,
            y: (point.y - self.rounding_origin.y) as f32 as f64 + rounding_offset_y,
        }
    }

    fn add_cubic(&mut self, p0: Point64, p1: Point64, p2: Point64, p3: Point64, depth: usize) {
        // The scanline package's Go `cubeFlatEnough` uses this squared
        // projection form. Keep it separate from the scene flattener's
        // normalized `controlsProgress` predicate so IEEE-754 operation order
        // remains source-shaped at the raster boundary.
        if cube_flat_enough_scanline(p0, p1, p2, p3, self.tolerance)
            || depth >= MAX_SCANLINE_CURVE_DEPTH
        {
            // Go rounds the cubic controls and endpoint when CubeTo64 is
            // submitted, but preserves the f64 midpoint generated while
            // flattening. Do not round this final chord a second time.
            let before = self.edges.len();
            self.add_edge_raw(p0, p3);
            if let Some(edge) = self.edges.get_mut(before) {
                edge.hull_y = Some((
                    p0.y.min(p1.y).min(p2.y).min(p3.y),
                    p0.y.max(p1.y).max(p2.y).max(p3.y),
                ));
            }
            return;
        }
        let p01 = midpoint(p0, p1);
        let p12 = midpoint(p1, p2);
        let p23 = midpoint(p2, p3);
        let p012 = midpoint(p01, p12);
        let p123 = midpoint(p12, p23);
        let middle = midpoint(p012, p123);
        self.add_cubic(p0, p01, p012, middle, depth + 1);
        self.add_cubic(middle, p123, p23, p3, depth + 1);
    }

    fn paint(&mut self, pixels: &mut [u8], color: [u8; 4]) {
        self.paint_with_mode(pixels, &mut |_x, _y| Some(color), false, Some(color));
    }

    fn paint_with<F>(&mut self, pixels: &mut [u8], mut color_at: F)
    where
        F: FnMut(usize, usize) -> Option<[u8; 4]>,
    {
        self.paint_with_mode(pixels, &mut color_at, false, None);
    }

    fn paint_with_go_blend<F>(&mut self, pixels: &mut [u8], mut color_at: F)
    where
        F: FnMut(usize, usize) -> Option<[u8; 4]>,
    {
        self.paint_with_mode(pixels, &mut color_at, true, None);
    }

    fn paint_with_mode<F>(
        &mut self,
        pixels: &mut [u8],
        color_at: &mut F,
        go_blend: bool,
        solid: Option<[u8; 4]>,
    ) where
        F: FnMut(usize, usize) -> Option<[u8; 4]>,
    {
        PAINT_ROW_PROBE.with(|probe| {
            if let Some((first, end)) = probe.get() {
                // Surface rows the edges span: paint_window writes no others.
                let reference_y = self.reference_origin.y.round();
                let (low, high) = self.edges().iter().fold(
                    (f64::INFINITY, f64::NEG_INFINITY),
                    |(low, high), edge| {
                        (
                            low.min(edge.from.y).min(edge.to.y),
                            high.max(edge.from.y).max(edge.to.y),
                        )
                    },
                );
                if low <= high {
                    let low = (low + reference_y).floor();
                    let high = (high + reference_y).ceil();
                    let (low, high) = if low.is_finite() && high.is_finite() {
                        (low as isize, high as isize)
                    } else {
                        (isize::MIN, isize::MAX)
                    };
                    probe.set(Some((first.min(low), end.max(high))));
                }
            }
        });
        if !go_band_recording()
            && let Some((top, bottom)) = frame_band_rows(self.surface_width, self.surface_height)
            && self.prepared.as_ref().is_some_and(|prepared| {
                prepared
                    .bounds
                    .is_none_or(|(_, first, _, end)| end <= top || first >= bottom)
            })
        {
            return;
        }
        if go_band_recording() {
            // Side buffers (shadow crops) use their own coordinates; only
            // surface rasterizers describe source primitive bounds.
            if go_band_frame_active(self.width, self.height) {
                self.record_go_leaf();
            }
            return;
        }
        if self.output.is_none() {
            self.output = layer_output(pixels);
        }
        let windows = go_band_windows(self.width, self.height).or_else(|| {
            // Root primitives have no effect-layer window, but RenderBands
            // still resets their rasterizer once per 256-row destination.
            // A full-frame active list is observably different because its
            // expired-edge swaps reorder f32 coverage accumulation.
            (GO_PNG_BANDS.with(std::cell::Cell::get)
                && go_band_frame_active(self.width, self.height))
            .then(|| {
                let rows = frame_band_rows(self.width, self.height);
                (0..self.height)
                    .step_by(GO_PNG_BAND_HEIGHT)
                    .filter_map(|top| {
                        let bottom = (top + GO_PNG_BAND_HEIGHT).min(self.height);
                        rows.is_none_or(|(first, end)| bottom > first && top < end)
                            .then_some((0, top as isize, self.width as isize, bottom as isize))
                    })
                    .collect()
            })
        });
        let Some(windows) = windows else {
            self.paint_window(pixels, color_at, go_blend, solid, None);
            return;
        };
        // Source d2cli renders PNG with RenderBands: every primitive is
        // rasterized once per destination (a 256-row band, or an effect layer
        // within it). Rasterizer coordinates start at that destination, so
        // translate by reference - destination exactly as setRasterizerReference
        // does, and scan only its rows and columns.
        let (width, height, reference) = (self.width, self.height, self.reference_origin);
        let reference_x = reference.x.round();
        let reference_y = reference.y.round();
        #[cfg(feature = "diagnostic-traces")]
        if raster_trace_pixel().is_some() {
            eprintln!(
                "d2-rs-raster go-band reference=({reference_x},{reference_y}) windows={windows:?} edges={}",
                self.edges().len()
            );
        }
        for (x0, y0, x1, y1) in windows {
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let offset = Point64 {
                x: x0 as f64 - reference_x,
                y: y0 as f64 - reference_y,
            };
            self.width = (x1 - x0) as usize;
            self.height = (y1 - y0) as usize;
            self.reference_origin = Point64 {
                x: x0 as f64,
                y: y0 as f64,
            };
            self.touched_min = self.width;
            self.touched_max = 0;
            self.paint_window(pixels, color_at, go_blend, solid, Some(offset));
        }
        self.width = width;
        self.height = height;
        self.reference_origin = reference;
        self.touched_min = width;
        self.touched_max = 0;
    }

    fn record_go_leaf(&mut self) {
        let (segments, expansion, full_expansion) = if let Some(prepared) = &self.prepared {
            (
                prepared.stroke_segments.clone(),
                prepared.stroke_expansion,
                prepared.stroke_full_expansion,
            )
        } else {
            (
                std::rc::Rc::new(std::mem::take(&mut self.stroke_segments)),
                self.stroke_expansion,
                self.stroke_full_expansion,
            )
        };
        let leaf = if GO_BAND_STROKE_ONLY.with(std::cell::Cell::get) && !segments.is_empty() {
            GoLeafBounds::Stroke {
                segments,
                expansion,
                full_expansion,
            }
        } else if !segments.is_empty() {
            // strokeRunPixelBounds: run points expanded by strokeExtent + 1
            // (a miter join scales the extent by its limit).
            let (mut l, mut t) = (f64::INFINITY, f64::INFINITY);
            let (mut r, mut b) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
            for (from, to) in segments.iter() {
                l = l.min(from.x).min(to.x);
                t = t.min(from.y).min(to.y);
                r = r.max(from.x).max(to.x);
                b = b.max(from.y).max(to.y);
            }
            let e = full_expansion;
            GoLeafBounds::Rect(l - e, t - e, r + e, b + e)
        } else {
            let reference_x = self.reference_origin.x.round();
            let reference_y = self.reference_origin.y.round();
            let (mut l, mut t) = (f64::INFINITY, f64::INFINITY);
            let (mut r, mut b) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
            for edge in self.edges() {
                for point in [edge.from, edge.to] {
                    l = l.min(point.x + reference_x);
                    t = t.min(point.y + reference_y);
                    r = r.max(point.x + reference_x);
                    b = b.max(point.y + reference_y);
                }
            }
            if l.is_infinite() {
                return;
            }
            // subpathPixelBounds' one-pixel antialiasing expansion.
            GoLeafBounds::Rect(l - 1.0, t - 1.0, r + 1.0, b + 1.0)
        };
        GO_BAND_RECORD.with(|record| {
            if let Some(record) = record.borrow_mut().last_mut() {
                record.push((leaf, None));
            }
        });
    }

    fn paint_window<F>(
        &mut self,
        pixels: &mut [u8],
        color_at: &mut F,
        go_blend: bool,
        solid: Option<[u8; 4]>,
        edge_offset: Option<Point64>,
    ) where
        F: FnMut(usize, usize) -> Option<[u8; 4]>,
    {
        if self.edges().is_empty() || self.width == 0 || self.height == 0 {
            return;
        }
        self.ensure_scan_buffers();
        // Rows written into a layer whose paint bounds are being tracked.
        let mut painted = PaintBounds::tracking(pixels);
        let reference_x = self.reference_origin.x.round() as isize;
        let reference_y = self.reference_origin.y.round() as isize;
        #[cfg(feature = "diagnostic-traces")]
        let trace_pixel = raster_trace_pixel().and_then(|(x, y)| {
            Some((
                x.checked_sub(self.trace_origin_x)?,
                y.checked_sub(self.trace_origin_y)?,
            ))
        });
        let sparse = solid.is_some() && self.edges().len() <= self.height.max(8);
        #[cfg(feature = "diagnostic-traces")]
        let sparse = sparse && trace_pixel.is_none();
        let (first, last) = self.prepare_scan_edges(edge_offset);
        // Row heads are set only for rows first..last (prepare_scan_edges).
        let head_rows = first.min(self.height)..last.max(first.min(self.height));
        self.active.clear();
        // A band pass writes only its own frame rows. Rows above the band are
        // still accumulated (edge state carries from row to row) but not
        // walked, and no row past its end can write.
        let band = frame_band_rows(self.surface_width, self.surface_height);
        // A compact output region bounds the written rows the same way.
        let band = match (band, &self.output) {
            (band, None) => band,
            (band, Some(region)) => {
                let (top, bottom) = (region.top, region.top + region.rows.len());
                Some(match band {
                    Some((band_top, band_bottom)) => (
                        top.max(band_top),
                        bottom.min(band_bottom).max(top.max(band_top)),
                    ),
                    None => (top, bottom),
                })
            }
        };
        let last = match band {
            // A fill that ends above the band writes nothing in this pass.
            Some((top, _)) if last as isize + reference_y <= top as isize => 0,
            Some((_, bottom)) => last.min((bottom as isize - reference_y).max(0) as usize),
            None => last,
        };
        for row in first.min(self.height)..last {
            // Match Go's scanline active-list removal: expired entries are
            // replaced with the last entry, rather than a stable retain.
            // The order is observable because partial/difference values are
            // accumulated as f32 and must follow the same edge sequence.
            let mut active_index = 0;
            while active_index < self.active.len() {
                if self.scan_edges[self.active[active_index]].end > row {
                    active_index += 1;
                    continue;
                }
                let last = self.active.pop().expect("active edge exists");
                if active_index < self.active.len() {
                    self.active[active_index] = last;
                }
            }
            let mut head = self.row_heads[row];
            self.row_heads[row] = 0;
            while head != 0 {
                let index = head - 1;
                self.active.push(index);
                head = self.scan_edges[index].next;
            }
            let global_y = row as isize + reference_y;
            let above_band = band.is_some_and(|(top, _)| global_y < top as isize);
            if above_band {
                // Nothing in this row is walked: only advance each edge
                // exactly as accumulate_edge_row does, without coverage.
                for active_index in 0..self.active.len() {
                    let index = self.active[active_index];
                    self.advance_edge_row(index, row);
                }
                continue;
            }
            for active_index in 0..self.active.len() {
                let index = self.active[active_index];
                self.accumulate_edge_row(index, row);
            }
            let mut winding = 0.0_f32;
            // This row's destination bytes and the surface columns they hold.
            let output_row = match &self.output {
                _ if above_band => None,
                None => (0..self.surface_height as isize)
                    .contains(&global_y)
                    .then(|| {
                        (
                            global_y as usize * self.surface_width * 4,
                            0,
                            self.surface_width,
                        )
                    }),
                Some(region) => region
                    .row(global_y)
                    .map(|(x0, width, offset)| (offset * 4, x0 as isize, width)),
            };
            // The coverage walk below only writes pixels of an output row;
            // edge accumulation above already advanced every active edge.
            let Some((row_start, output_x0, output_width)) = output_row else {
                self.clear_touched();
                continue;
            };
            if let Some(bounds) = &mut painted {
                // The walk writes only columns touched_min..touched_max.
                let x0 = (self.touched_min as isize + reference_x).max(output_x0);
                let x1 = (self.touched_max as isize + reference_x)
                    .min(output_x0 + output_width as isize);
                if x0 < x1 {
                    bounds.add_row(global_y as usize, x0 as usize, x1 as usize);
                }
            }
            // Source rasterize/WalkCoverage visits the exact accumulation
            // interval, including prefix difference events at the left edge.
            let mut x = self.touched_min;
            // Columns at or past this end write nothing; only the running
            // winding to their left matters.
            let output_end = (output_x0 + output_width as isize - reference_x)
                .clamp(0, self.touched_max as isize) as usize;
            while x < output_end {
                winding += self.difference[x];
                let area = self.partial[x] + winding;
                let pixel_coverage = coverage(area);
                let global_x = x as isize + reference_x;
                let output_x = global_x - output_x0;
                if pixel_coverage != 0
                    && (0..output_width as isize).contains(&output_x)
                    && let Some(color) = color_at(global_x as usize, global_y as usize)
                    && color[3] != 0
                {
                    let pixel_start = row_start + output_x as usize * 4;
                    let pixel = &mut pixels[pixel_start..pixel_start + 4];
                    #[cfg(feature = "diagnostic-traces")]
                    let before = (trace_pixel == Some((global_x as usize, global_y as usize)))
                        .then(|| [pixel[0], pixel[1], pixel[2], pixel[3]]);
                    #[cfg(feature = "diagnostic-traces")]
                    if before.is_some() && self.edges().len() > 100 {
                        for (index, edge) in self.edges().iter().enumerate() {
                            eprintln!(
                                "d2-rs-raster edge index={index} from=({:.17e},{:.17e}) from_bits=({:016x},{:016x}) to=({:.17e},{:.17e}) to_bits=({:016x},{:016x})",
                                edge.from.x,
                                edge.from.y,
                                edge.from.x.to_bits(),
                                edge.from.y.to_bits(),
                                edge.to.x,
                                edge.to.y,
                                edge.to.x.to_bits(),
                                edge.to.y.to_bits(),
                            );
                        }
                    }
                    #[cfg(feature = "diagnostic-traces")]
                    if before.is_some() && self.edges().len() == 3 {
                        for (index, edge) in self.edges().iter().enumerate() {
                            eprintln!(
                                "d2-rs-raster edge index={index} from=({:.17e},{:.17e}) from_bits=({:016x},{:016x}) to=({:.17e},{:.17e}) to_bits=({:016x},{:016x})",
                                edge.from.x,
                                edge.from.y,
                                edge.from.x.to_bits(),
                                edge.from.y.to_bits(),
                                edge.to.x,
                                edge.to.y,
                                edge.to.x.to_bits(),
                                edge.to.y.to_bits(),
                            );
                        }
                    }
                    #[cfg(feature = "diagnostic-traces")]
                    if before.is_some() {
                        eprintln!(
                            "d2-rs-raster accum x={} y={} edges={} origin=({:.17e},{:.17e}) partial={:.9} partial_bits={:08x} difference={:.9} difference_bits={:08x} winding={:.9} winding_bits={:08x}",
                            global_x,
                            global_y,
                            self.edges().len(),
                            self.reference_origin.x,
                            self.reference_origin.y,
                            self.partial[x],
                            self.partial[x].to_bits(),
                            self.difference[x],
                            self.difference[x].to_bits(),
                            winding,
                            winding.to_bits(),
                        );
                        for index in &self.active {
                            let edge = &self.scan_edges[*index];
                            eprintln!(
                                "d2-rs-raster scan-edge index={index} end={} x={:.17e} x_bits={:016x} y={:.17e} y_bits={:016x} bottom={:.17e} slope={:.17e} slope_bits={:016x} inverse_slope={:.17e} winding={}",
                                edge.end,
                                edge.x + reference_x as f64,
                                (edge.x + reference_x as f64).to_bits(),
                                edge.y + reference_y as f64,
                                (edge.y + reference_y as f64).to_bits(),
                                edge.bottom + reference_y as f64,
                                edge.slope,
                                edge.slope.to_bits(),
                                edge.inverse_slope,
                                edge.winding,
                            );
                        }
                    }
                    if go_blend {
                        blend_pixel_go(pixel, color, pixel_coverage);
                    } else {
                        blend_pixel(pixel, color, pixel_coverage);
                    }
                    #[cfg(feature = "diagnostic-traces")]
                    if let Some(before) = before {
                        eprintln!(
                            "d2-rs-raster paint x={} y={} layer_origin=({}, {}) edges={} origin=({:.17e},{:.17e}) area={area:.9} area_bits={:08x} coverage={pixel_coverage} go_blend={go_blend} color={color:?} before={before:?} after={:?}",
                            global_x,
                            global_y,
                            self.trace_origin_x,
                            self.trace_origin_y,
                            self.edges().len(),
                            self.reference_origin.x,
                            self.reference_origin.y,
                            area.to_bits(),
                            [pixel[0], pixel[1], pixel[2], pixel[3]],
                        );
                    }
                }
                x += 1;
                // Source DrawRGBA consumes event-free interiors once, reusing
                // constant coverage/source-over coefficients (or packed fill).
                if sparse
                    && x < self.touched_max
                    && zero_coverage_values(self.partial[x], self.difference[x])
                {
                    let end = (self.touched_min + ((x - self.touched_min) / 4096 + 1) * 4096)
                        .min(self.touched_max);
                    let run_end = zero_coverage_run_end(&self.partial, &self.difference, x, end);
                    let span_coverage = coverage(winding);
                    if span_coverage != 0 {
                        let left = (reference_x + x as isize - output_x0)
                            .clamp(0, output_width as isize)
                            as usize;
                        let right = (reference_x + run_end as isize - output_x0)
                            .clamp(0, output_width as isize)
                            as usize;
                        paint_solid_span(
                            &mut pixels[row_start + left * 4..row_start + right * 4],
                            solid.unwrap(),
                            span_coverage,
                        );
                    }
                    x = run_end;
                }
            }
            self.clear_touched();
        }
        if let Some(bounds) = painted {
            bounds.commit();
        }
        // Clear the heads of rows this pass did not consume, so the buffer
        // is all zero again for the next paint (and the scan buffer pool).
        self.row_heads[head_rows].fill(0);
    }

    /// Requires all row heads to be zero: fresh scan buffers are, and every
    /// paint clears the heads it set (paint_window).
    fn prepare_scan_edges(&mut self, offset: Option<Point64>) -> (usize, usize) {
        self.scan_edges.clear();
        // Go prepareScanEdges returns these bounds as it builds the list.
        let (mut first, mut last) = (self.height, 0);
        let prepared = self.prepared.clone();
        let edges = prepared
            .as_ref()
            .map_or(self.edges.as_slice(), |p| p.edges.as_slice());
        for (block_index, block) in edges.chunks(PREPARED_EDGE_BLOCK).enumerate() {
            if let Some(offset) = offset
                && let Some(&(top, bottom)) = prepared
                    .as_ref()
                    .and_then(|p| p.edge_blocks.get(block_index))
                && (bottom - offset.y <= 0.0 || top - offset.y >= self.height as f64)
            {
                continue;
            }
            for edge in block {
                // bands.go:setRasterizerReference translates the reference-rounded
                // path into each destination. Consume the immutable prepared edges
                // directly, retaining the same subtractions and source edge order
                // without constructing a translated edge slice for every window.
                let (from, to) = if let Some(offset) = offset {
                    if !edge.hull_y.is_none_or(|(top, bottom)| {
                        bottom - offset.y > 0.0 && top - offset.y < self.height as f64
                    }) {
                        continue;
                    }
                    (
                        Point64 {
                            x: edge.from.x - offset.x,
                            y: edge.from.y - offset.y,
                        },
                        Point64 {
                            x: edge.to.x - offset.x,
                            y: edge.to.y - offset.y,
                        },
                    )
                } else {
                    (edge.from, edge.to)
                };
                let (from, to, winding) = if from.y <= to.y {
                    (from, to, 1.0)
                } else {
                    (to, from, -1.0)
                };
                if to.y <= 0.0 || from.y >= self.height as f64 {
                    continue;
                }
                let start = from.y.max(0.0).floor() as usize;
                let end = to.y.min(self.height as f64).ceil() as usize;
                if start >= end || to.y == from.y {
                    continue;
                }
                let slope = (to.x - from.x) / (to.y - from.y);
                let top = from.y.max(start as f64);
                let x = (top - from.y).mul_add(slope, from.x);
                let inverse_slope = if slope == 0.0 { 0.0 } else { 1.0 / slope };
                let index = self.scan_edges.len();
                self.scan_edges.push(ScanEdge {
                    end,
                    next: self.row_heads[start.min(self.height.saturating_sub(1))],
                    x,
                    y: top,
                    bottom: to.y,
                    slope,
                    inverse_slope,
                    winding,
                });
                if start < self.height {
                    self.row_heads[start] = index + 1;
                }
                first = first.min(start);
                last = last.max(end);
            }
        }
        (first, last)
    }

    /// The edge-state part of accumulate_edge_row: move the edge to the end
    /// of `row` (or its bottom) without accumulating coverage.
    fn advance_edge_row(&mut self, index: usize, row: usize) {
        let active = &mut self.scan_edges[index];
        let bottom = active.bottom.min((row + 1) as f64);
        let height = bottom - active.y;
        if height <= 0.0 {
            return;
        }
        active.x = height.mul_add(active.slope, active.x);
        active.y = bottom;
    }

    fn accumulate_edge_row(&mut self, index: usize, row: usize) {
        let active = &mut self.scan_edges[index];
        let bottom = active.bottom.min((row + 1) as f64);
        let height = bottom - active.y;
        if height <= 0.0 {
            return;
        }
        let x_top = active.x;
        let x_bottom = height.mul_add(active.slope, x_top);
        let top = active.y;
        active.x = x_bottom;
        active.y = bottom;
        let sign = active.winding;
        let delta_x = x_bottom - x_top;
        if delta_x == 0.0 {
            if x_top < 0.0 {
                self.add_difference(0, sign * height);
            } else if x_top >= self.width as f64 {
                self.add_difference(self.width, sign * height);
            } else {
                let pixel = x_top.floor() as usize;
                self.accumulate_visible(pixel, sign, x_top, x_bottom, height);
                self.touch(pixel, pixel + 1);
            }
            return;
        }
        let dy_per_x = active.inverse_slope;
        let mut current_x = x_top;
        let mut current_y = top;
        if delta_x > 0.0 {
            if current_x < 0.0 {
                if x_bottom <= 0.0 {
                    self.add_difference(0, sign * height);
                    return;
                }
                let cross_y = (-current_x).mul_add(dy_per_x, current_y);
                self.add_difference(0, sign * (cross_y - current_y));
                current_x = 0.0;
                current_y = cross_y;
            }
            if current_x >= self.width as f64 {
                self.add_difference(self.width, sign * (bottom - current_y));
                return;
            }
            let visible_end_x = x_bottom.min(self.width as f64);
            let visible_end_y = (visible_end_x - current_x).mul_add(dy_per_x, current_y);
            self.touch(
                current_x.floor() as usize,
                self.width.min(visible_end_x.floor() as usize + 1),
            );
            let mut boundary = current_x.floor() + 1.0;
            if boundary < visible_end_x {
                let next_y = (boundary - current_x).mul_add(dy_per_x, current_y);
                self.accumulate_visible(
                    boundary as usize - 1,
                    sign,
                    current_x,
                    boundary,
                    next_y - current_y,
                );
                current_x = boundary;
                current_y = next_y;
                boundary += 1.0;
                let partial = (sign * dy_per_x * 0.5) as f32;
                let difference = (sign * dy_per_x) as f32;
                while boundary < visible_end_x {
                    let pixel = boundary as usize - 1;
                    self.partial[pixel] += partial;
                    self.difference[pixel + 1] += difference;
                    current_x = boundary;
                    current_y += dy_per_x;
                    boundary += 1.0;
                }
            }
            self.accumulate_visible(
                ((current_x + visible_end_x) * 0.5).floor() as usize,
                sign,
                current_x,
                visible_end_x,
                visible_end_y - current_y,
            );
            if visible_end_y < bottom {
                self.add_difference(self.width, sign * (bottom - visible_end_y));
            }
            return;
        }
        if current_x > self.width as f64 {
            if x_bottom >= self.width as f64 {
                self.add_difference(self.width, sign * height);
                return;
            }
            let cross_y = ((self.width as f64) - current_x).mul_add(dy_per_x, current_y);
            self.add_difference(self.width, sign * (cross_y - current_y));
            current_x = self.width as f64;
            current_y = cross_y;
        }
        if current_x <= 0.0 {
            self.add_difference(0, sign * (bottom - current_y));
            return;
        }
        let visible_end_x = x_bottom.max(0.0);
        let visible_end_y = (visible_end_x - current_x).mul_add(dy_per_x, current_y);
        self.touch(
            visible_end_x.floor() as usize,
            self.width.min(current_x.floor() as usize + 1),
        );
        let mut boundary = current_x.ceil() - 1.0;
        if boundary > visible_end_x {
            let next_y = (boundary - current_x).mul_add(dy_per_x, current_y);
            self.accumulate_visible(
                boundary as usize,
                sign,
                current_x,
                boundary,
                next_y - current_y,
            );
            current_x = boundary;
            current_y = next_y;
            boundary -= 1.0;
            let full_height = -dy_per_x;
            let partial = (sign * full_height * 0.5) as f32;
            let difference = (sign * full_height) as f32;
            while boundary > visible_end_x {
                let pixel = boundary as usize;
                self.partial[pixel] += partial;
                self.difference[pixel + 1] += difference;
                current_x = boundary;
                current_y += full_height;
                boundary -= 1.0;
            }
        }
        self.accumulate_visible(
            ((current_x + visible_end_x) * 0.5).floor() as usize,
            sign,
            current_x,
            visible_end_x,
            visible_end_y - current_y,
        );
        if visible_end_y < bottom {
            self.add_difference(0, sign * (bottom - visible_end_y));
        }
    }

    fn accumulate_visible(&mut self, pixel: usize, sign: f64, from_x: f64, to_x: f64, height: f64) {
        if height <= 0.0 || pixel >= self.width {
            return;
        }
        let middle_x = (from_x + to_x) * 0.5;
        self.partial[pixel] += (sign * height * ((pixel + 1) as f64 - middle_x)) as f32;
        self.difference[pixel + 1] += (sign * height) as f32;
    }

    fn add_difference(&mut self, x: usize, amount: f64) {
        if amount == 0.0 {
            return;
        }
        let x = x.min(self.width);
        self.difference[x] += amount as f32;
        self.touch(x, x);
    }

    // Original internal/scanline/rasterizer.go touch/touchEvent/clearTouched.
    // The final difference event is retained for clearing even when no visible
    // pixel was touched (e.g. an edge entirely to the right of the destination).
    fn touch(&mut self, min: usize, max: usize) {
        self.touched = true;
        self.touched_min = self.touched_min.min(min);
        self.touched_max = self.touched_max.max(max);
    }

    fn clear_touched(&mut self) {
        if !self.touched {
            return;
        }
        let end = self.touched_max.min(self.width);
        if self.touched_min < end {
            self.partial[self.touched_min..end].fill(0.0);
        }
        self.difference[self.touched_min..=end].fill(0.0);
        self.touched_min = self.width;
        self.touched_max = 0;
        self.touched = false;
    }
}

fn map_point(point: Point64, transform: Affine64) -> Point64 {
    Point64 {
        x: point.x * transform.sx + point.y * transform.kx + transform.tx,
        y: point.x * transform.ky + point.y * transform.sy + transform.ty,
    }
}

fn canonical_transform(transform: Transform) -> Affine64 {
    // usvg stores affine values as f32. D2 keeps scene transforms in f64 and
    // rounds only when submitting scanline coordinates; recover decimal
    // layout translations that are otherwise one f32 ulp away from the Go
    // scene values (for example marker translations such as 220.4).
    let snap = |value: f32| -> f64 { (f64::from(value) * 10_000.0).round() / 10_000.0 };
    Affine64 {
        sx: f64::from(transform.sx),
        kx: f64::from(transform.kx),
        ky: f64::from(transform.ky),
        sy: f64::from(transform.sy),
        tx: snap(transform.tx),
        ty: snap(transform.ty),
    }
}

// Computed glyph placements are binary fixed-point coordinates, not SVG
// decimal translations. Preserve every stored f32 bit when undoing them.
fn glyph_transform64(transform: Transform) -> Affine64 {
    Affine64 {
        sx: f64::from(transform.sx),
        kx: f64::from(transform.kx),
        ky: f64::from(transform.ky),
        sy: f64::from(transform.sy),
        tx: f64::from(transform.tx),
        ty: f64::from(transform.ty),
    }
}

fn affine_mul(left: Affine64, right: Affine64) -> Affine64 {
    Affine64 {
        sx: left.sx * right.sx + left.kx * right.ky,
        kx: left.sx * right.kx + left.kx * right.sy,
        ky: left.ky * right.sx + left.sy * right.ky,
        sy: left.ky * right.kx + left.sy * right.sy,
        tx: left.sx * right.tx + left.kx * right.ty + left.tx,
        ty: left.ky * right.tx + left.sy * right.ty + left.ty,
    }
}

// Go d2scene.Matrix.Mul uses an FMA for the first product on ARM64.
fn source_affine_mul(left: Affine64, right: Affine64) -> Affine64 {
    let m = crate::svg_precision::source_matrix_mul(
        [left.sx, left.ky, left.kx, left.sy, left.tx, left.ty],
        [right.sx, right.ky, right.kx, right.sy, right.tx, right.ty],
    );
    Affine64 {
        sx: m[0],
        ky: m[1],
        kx: m[2],
        sy: m[3],
        tx: m[4],
        ty: m[5],
    }
}

fn affine_inverse(transform: Affine64) -> Option<Affine64> {
    let determinant = transform.sx * transform.sy - transform.kx * transform.ky;
    if determinant == 0.0 || !determinant.is_finite() {
        return None;
    }
    Some(Affine64 {
        sx: transform.sy / determinant,
        kx: -transform.kx / determinant,
        ky: -transform.ky / determinant,
        sy: transform.sx / determinant,
        tx: (transform.kx * transform.ty - transform.sy * transform.tx) / determinant,
        ty: (transform.ky * transform.tx - transform.sx * transform.ty) / determinant,
    })
}

fn synthetic_italic_local_f32(x: f64, y: f64) -> Transform {
    // svgtypes composes the serialized transform list in f64 before usvg
    // casts the complete matrix to f32. Casting each factor before composing
    // gives a different translation and fails the private-wrapper guard.
    let parsed = synthetic_italic_local_f64(x, y);
    Transform::from_row(
        parsed.sx as f32,
        parsed.ky as f32,
        parsed.kx as f32,
        parsed.sy as f32,
        parsed.tx as f32,
        parsed.ty as f32,
    )
}

fn synthetic_italic_local_f64(x: f64, y: f64) -> Affine64 {
    let skew = (-12.0_f64.to_radians()).tan();
    Affine64 {
        sx: 1.0,
        kx: skew,
        ky: 0.0,
        sy: 1.0,
        // Translate(x,y) * SkewX(angle) * Translate(-x,-y), in
        // d2scene.Matrix.Mul's operation order. The x translations cancel;
        // retaining the first x alone shifts nonzero-origin runs twice.
        tx: -x + skew * -y + x,
        ty: 0.0,
    }
}

fn synthetic_italic_device_transform(
    device_transform: Transform,
    x: f64,
    y: f64,
) -> Option<Affine64> {
    let parsed = canonical_transform(synthetic_italic_local_f32(x, y));
    let exact = synthetic_italic_local_f64(x, y);
    let correction = affine_mul(affine_inverse(parsed)?, exact);
    Some(affine_mul(
        canonical_transform(device_transform),
        correction,
    ))
}

fn synthetic_italic_pivot(text: &usvg::Text) -> Option<(f64, f64)> {
    let id = text.id();
    let marker = id
        .split_once("--")
        .map(|(marker, _)| marker)
        .unwrap_or(id)
        .strip_prefix("d2rs-synthetic-italic-")?;
    let (x, y) = marker.split_once('-')?;
    let x = u64::from_str_radix(x, 16).ok().map(f64::from_bits)?;
    let y = u64::from_str_radix(y, 16).ok().map(f64::from_bits)?;
    Some((x, y))
}

fn signed_area(points: &[Point64]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.x * b.y - a.y * b.x)
        .sum::<f64>()
        * 0.5
}

fn midpoint(a: Point64, b: Point64) -> Point64 {
    Point64 {
        x: a.x + (b.x - a.x) * 0.5,
        y: a.y + (b.y - a.y) * 0.5,
    }
}

fn source_curve_coordinate(value: f32) -> f64 {
    let value = f64::from(value);
    (value * 1_000_000.0).round() / 1_000_000.0
}

// The retained Go text path flattener uses direct coordinate averaging,
// whereas the scanline path kernel uses the difference form above. Keep the
// text-specific operation separate so its last-bit IEEE-754 results match
// d2raster's geometry.go without perturbing ordinary scene paths.
fn midpoint_scene(a: Point64, b: Point64) -> Point64 {
    Point64 {
        // Keep the operation order used by d2raster's geometry.go. The
        // seemingly equivalent multiply form can differ by one IEEE-754 bit
        // at glyph curves that are subsequently skewed.
        x: (a.x + b.x) / 2.0,
        y: (a.y + b.y) / 2.0,
    }
}

fn cube_flat_enough(p0: Point64, p1: Point64, p2: Point64, p3: Point64, tolerance: f64) -> bool {
    let dx = p3.x - p0.x;
    let dy = p3.y - p0.y;
    let length = dx.hypot(dy);
    if length == 0.0 {
        return [p1, p2].into_iter().all(|control| {
            let dx = control.x - p0.x;
            let dy = control.y - p0.y;
            dx.hypot(dy) <= tolerance
        });
    }
    let unit_x = dx / length;
    let unit_y = dy / length;
    let mut previous_projection = 0.0;
    for control in [p1, p2] {
        let control_x = control.x - p0.x;
        let control_y = control.y - p0.y;
        // ARM64 geometry.go:400–401: round the first product, then fuse
        // the second. A one-ulp change at tolerance must subdivide the curve.
        let projection = d2_rs_render::go_float::go_fmadd(control_y, unit_y, control_x * unit_x);
        let perpendicular =
            d2_rs_render::go_float::go_fmsub(control_y, unit_x, control_x * unit_y).abs();
        let excess = if projection < 0.0 {
            -projection
        } else if projection > length {
            projection - length
        } else {
            0.0
        };
        if perpendicular.hypot(excess) > tolerance || projection < previous_projection - tolerance {
            return false;
        }
        previous_projection = projection;
    }
    true
}

fn cube_flat_enough_scanline(
    p0: Point64,
    p1: Point64,
    p2: Point64,
    p3: Point64,
    tolerance: f64,
) -> bool {
    let dx = p3.x - p0.x;
    let dy = p3.y - p0.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared == 0.0 {
        let d1x = p1.x - p0.x;
        let d1y = p1.y - p0.y;
        let d2x = p2.x - p0.x;
        let d2y = p2.y - p0.y;
        return (d1x * d1x + d1y * d1y).max(d2x * d2x + d2y * d2y) <= tolerance * tolerance;
    }
    let limit = tolerance * tolerance * length_squared;
    let projection_tolerance = tolerance * length_squared.sqrt();
    let mut previous_projection = 0.0;
    for control in [p1, p2] {
        let control_x = control.x - p0.x;
        let control_y = control.y - p0.y;
        let projection = control_x * dx + control_y * dy;
        let cross = dx * control_y - dy * control_x;
        let excess = if projection < 0.0 {
            -projection
        } else if projection > length_squared {
            projection - length_squared
        } else {
            0.0
        };
        if cross * cross + excess * excess > limit
            || projection < previous_projection - projection_tolerance
        {
            return false;
        }
        previous_projection = projection;
    }
    true
}

fn collect_centerline(path: &Path, tolerance: f64) -> Option<(Vec<Point64>, bool)> {
    if let Some(centerline) = collect_rounded_rect_centerline(path, tolerance) {
        return Some(centerline);
    }
    let mut points = Vec::new();
    let mut current = None;
    let mut subpaths = 0;
    let mut closed = false;
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(point) => {
                subpaths += 1;
                if subpaths > 1 {
                    return None;
                }
                let point = Point64 {
                    x: f64::from(point.x),
                    y: f64::from(point.y),
                };
                points.push(point);
                current = Some(point);
                closed = false;
            }
            PathSegment::LineTo(point) => {
                let point = Point64 {
                    x: f64::from(point.x),
                    y: f64::from(point.y),
                };
                points.push(point);
                current = Some(point);
            }
            PathSegment::CubicTo(c0, c1, point) => {
                let Some(from) = current else { return None };
                let c0 = Point64 {
                    x: source_curve_coordinate(c0.x),
                    y: source_curve_coordinate(c0.y),
                };
                let c1 = Point64 {
                    x: source_curve_coordinate(c1.x),
                    y: source_curve_coordinate(c1.y),
                };
                let to = Point64 {
                    x: source_curve_coordinate(point.x),
                    y: source_curve_coordinate(point.y),
                };
                flatten_centerline(from, c0, c1, to, tolerance, 0, &mut points);
                current = Some(to);
            }
            PathSegment::QuadTo(_, _) => return None,
            PathSegment::Close => {
                closed = true;
                current = points.first().copied();
            }
        }
    }
    (points.len() >= 2).then_some((points, closed))
}

fn collect_raster_source_subpaths(
    source: &d2_rs_render::RasterPathSource,
    tolerance: f64,
) -> Option<Vec<(Vec<Point64>, bool)>> {
    use d2_rs_render::RasterPathCommand;

    let mut subpaths = Vec::new();
    let mut points = Vec::new();
    let mut current = None;
    let mut first = None;
    let mut closed = false;
    let finish_subpath =
        |subpaths: &mut Vec<(Vec<Point64>, bool)>, points: &mut Vec<Point64>, closed: bool| {
            if points.len() >= 2 {
                subpaths.push((std::mem::take(points), closed));
            } else {
                points.clear();
            }
        };
    for command in &source.commands {
        match *command {
            RasterPathCommand::MoveTo(point) => {
                finish_subpath(&mut subpaths, &mut points, closed);
                let point = Point64 {
                    x: point.x,
                    y: point.y,
                };
                points.push(point);
                current = Some(point);
                first = Some(point);
                closed = false;
            }
            RasterPathCommand::LineTo(point) => {
                let point = Point64 {
                    x: point.x,
                    y: point.y,
                };
                points.push(point);
                current = Some(point);
            }
            RasterPathCommand::CubicTo(control0, control1, end) => {
                let Some(from) = current else { return None };
                let control0 = Point64 {
                    x: control0.x,
                    y: control0.y,
                };
                let control1 = Point64 {
                    x: control1.x,
                    y: control1.y,
                };
                let end = Point64 { x: end.x, y: end.y };
                flatten_centerline(from, control0, control1, end, tolerance, 0, &mut points);
                current = Some(end);
            }
            RasterPathCommand::Close => {
                closed = true;
                current = first;
            }
        }
    }
    finish_subpath(&mut subpaths, &mut points, closed);
    (!subpaths.is_empty()).then_some(subpaths)
}

/// usvg lowers SVG `<rect rx ry>` elements to float32 cubic controls before
/// exposing the path. D2's importer retains the rectangle primitive and the
/// Go rasterizer rebuilds those controls from the canonical kappa constant in
/// f64. Recover that common rounded-rectangle topology so imported gradients
/// use the same edge coordinates instead of inheriting the float32 controls.
fn collect_scene_rect_centerline(rect: NativeRect, tolerance: f64) -> (Vec<Point64>, bool) {
    // Source geometry.go::roundedRectSubpaths. Retain its contour origin and
    // cubic arithmetic; a contour rotation changes every closed dash break.
    let [x, y, width, height] = rect.box_values;
    let rx = rect.radii[0].min(width / 2.0);
    let ry = rect.radii[1].min(height / 2.0);
    let right = x + width;
    let bottom = y + height;
    if rx == 0.0 || ry == 0.0 {
        return (
            vec![
                Point64 { x, y },
                Point64 { x: right, y },
                Point64 {
                    x: right,
                    y: bottom,
                },
                Point64 { x, y: bottom },
            ],
            true,
        );
    }
    let k = 0.5522847498307936;
    let curves = [
        [
            Point64 { x: right - rx, y },
            Point64 {
                x: right - rx + k * rx,
                y,
            },
            Point64 {
                x: right,
                y: y + ry - k * ry,
            },
            Point64 {
                x: right,
                y: y + ry,
            },
        ],
        [
            Point64 {
                x: right,
                y: bottom - ry,
            },
            Point64 {
                x: right,
                y: bottom - ry + k * ry,
            },
            Point64 {
                x: right - rx + k * rx,
                y: bottom,
            },
            Point64 {
                x: right - rx,
                y: bottom,
            },
        ],
        [
            Point64 {
                x: x + rx,
                y: bottom,
            },
            Point64 {
                x: x + rx - k * rx,
                y: bottom,
            },
            Point64 {
                x,
                y: bottom - ry + k * ry,
            },
            Point64 { x, y: bottom - ry },
        ],
        [
            Point64 { x, y: y + ry },
            Point64 {
                x,
                y: y + ry - k * ry,
            },
            Point64 {
                x: x + rx - k * rx,
                y,
            },
            Point64 { x: x + rx, y },
        ],
    ];
    let mut points = vec![Point64 { x: x + rx, y }, curves[0][0]];
    for (index, [start, control0, control1, end]) in curves.into_iter().enumerate() {
        if index != 0 {
            points.push(start);
        }
        flatten_centerline(start, control0, control1, end, tolerance, 0, &mut points);
    }
    points.dedup();
    (points, true)
}

fn collect_rounded_rect_centerline(path: &Path, tolerance: f64) -> Option<(Vec<Point64>, bool)> {
    let segments = path.segments().collect::<Vec<_>>();
    let geometry = match segments.as_slice() {
        [
            PathSegment::MoveTo(m),
            PathSegment::LineTo(top_right),
            PathSegment::CubicTo(_, _, right_top),
            PathSegment::LineTo(right_bottom),
            PathSegment::CubicTo(_, _, bottom_right),
            PathSegment::LineTo(bottom_left),
            PathSegment::CubicTo(_, _, left_bottom),
            PathSegment::LineTo(left_top),
            PathSegment::CubicTo(_, _, close),
            PathSegment::Close,
        ] => Some((
            *m,
            *top_right,
            *right_top,
            *right_bottom,
            *bottom_right,
            *bottom_left,
            *left_bottom,
            *left_top,
            *close,
        )),
        [
            PathSegment::MoveTo(m),
            PathSegment::LineTo(top_right),
            PathSegment::CubicTo(_, _, _),
            PathSegment::CubicTo(_, _, right_top),
            PathSegment::LineTo(right_bottom),
            PathSegment::CubicTo(_, _, bottom_right),
            PathSegment::LineTo(bottom_left),
            PathSegment::CubicTo(_, _, left_bottom),
            PathSegment::LineTo(left_top),
            PathSegment::CubicTo(_, _, _),
            PathSegment::CubicTo(_, _, close),
            PathSegment::Close,
        ] => Some((
            *m,
            *top_right,
            *right_top,
            *right_bottom,
            *bottom_right,
            *bottom_left,
            *left_bottom,
            *left_top,
            *close,
        )),
        _ => None,
    }?;
    let (
        m,
        top_right,
        right_top,
        right_bottom,
        bottom_right,
        bottom_left,
        left_bottom,
        left_top,
        close,
    ) = geometry;
    let left = f64::from(left_bottom.x);
    let right = f64::from(right_top.x);
    let top = f64::from(m.y);
    let bottom = f64::from(bottom_right.y);
    let radius_x = f64::from(m.x) - left;
    let radius_y = f64::from(right_top.y) - top;
    if ![left, right, top, bottom, radius_x, radius_y]
        .into_iter()
        .all(|value| value.is_finite())
        || right <= left
        || bottom <= top
        || radius_x <= 0.0
        || radius_y <= 0.0
        || radius_x * 2.0 > right - left
        || radius_y * 2.0 > bottom - top
    {
        return None;
    }
    let close_matches = |point: resvg::tiny_skia::Point, x: f64, y: f64| {
        (f64::from(point.x) - x).abs() <= 1e-4 && (f64::from(point.y) - y).abs() <= 1e-4
    };
    if !close_matches(top_right, right - radius_x, top)
        || !close_matches(right_bottom, right, bottom - radius_y)
        || !close_matches(bottom_left, left + radius_x, bottom)
        || !close_matches(left_top, left, top + radius_y)
        || !close_matches(left_bottom, left, bottom - radius_y)
        || !close_matches(close, left + radius_x, top)
    {
        return None;
    }
    let k = 0.552_284_749_830_793_6;
    let curves = [
        (
            Point64 {
                x: right - radius_x,
                y: top,
            },
            Point64 {
                x: right - radius_x + k * radius_x,
                y: top,
            },
            Point64 {
                x: right,
                y: top + radius_y - k * radius_y,
            },
            Point64 {
                x: right,
                y: top + radius_y,
            },
        ),
        (
            Point64 {
                x: right,
                y: bottom - radius_y,
            },
            Point64 {
                x: right,
                y: bottom - radius_y + k * radius_y,
            },
            Point64 {
                x: right - radius_x + k * radius_x,
                y: bottom,
            },
            Point64 {
                x: right - radius_x,
                y: bottom,
            },
        ),
        (
            Point64 {
                x: left + radius_x,
                y: bottom,
            },
            Point64 {
                x: left + radius_x - k * radius_x,
                y: bottom,
            },
            Point64 {
                x: left,
                y: bottom - radius_y + k * radius_y,
            },
            Point64 {
                x: left,
                y: bottom - radius_y,
            },
        ),
        (
            Point64 {
                x: left,
                y: top + radius_y,
            },
            Point64 {
                x: left,
                y: top + radius_y - k * radius_y,
            },
            Point64 {
                x: left + radius_x - k * radius_x,
                y: top,
            },
            Point64 {
                x: left + radius_x,
                y: top,
            },
        ),
    ];
    let mut points = vec![
        curves[0].0,
        Point64 {
            x: f64::from(top_right.x),
            y: f64::from(top_right.y),
        },
    ];
    for (index, (start, control0, control1, end)) in curves.into_iter().enumerate() {
        if index > 0 {
            let line_end = match index {
                1 => right_bottom,
                2 => bottom_left,
                3 => left_top,
                _ => unreachable!(),
            };
            points.push(Point64 {
                x: f64::from(line_end.x),
                y: f64::from(line_end.y),
            });
        }
        flatten_centerline(start, control0, control1, end, tolerance, 0, &mut points);
    }
    Some((points, true))
}

fn collect_subpaths(path: &Path, tolerance: f64, quantize: bool) -> Vec<(Vec<Point64>, bool)> {
    let point64 = |point: resvg::tiny_skia::Point| {
        let mut x = f64::from(point.x);
        let mut y = f64::from(point.y);
        if quantize {
            // D2's sfnt.LoadGlyph exposes outline coordinates as 26.6 fixed
            // point values before flattening. usvg keeps the same font outline
            // in f32, so round only this text path back to the Go boundary.
            x = (x * 64.0).round() / 64.0;
            y = (y * 64.0).round() / 64.0;
        }
        Point64 { x, y }
    };
    let mut result = Vec::new();
    let mut points = Vec::new();
    let mut first = None;
    let mut current = None;
    let mut closed = false;
    let flush =
        |result: &mut Vec<(Vec<Point64>, bool)>, points: &mut Vec<Point64>, closed: bool| {
            if points.len() >= 2 {
                result.push((std::mem::take(points), closed));
            } else {
                points.clear();
            }
        };
    let append = |points: &mut Vec<Point64>, point: Point64| {
        if points
            .last()
            .is_none_or(|previous| previous.x != point.x || previous.y != point.y)
        {
            points.push(point);
        }
    };
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(point) => {
                flush(&mut result, &mut points, closed);
                let point = point64(point);
                first = Some(point);
                current = Some(point);
                closed = false;
                append(&mut points, point);
            }
            PathSegment::LineTo(point) => {
                let point = point64(point);
                append(&mut points, point);
                current = Some(point);
            }
            PathSegment::QuadTo(control, point) => {
                let Some(from) = current else { continue };
                let control = point64(control);
                let to = point64(point);
                flatten_quadratic(from, control, to, tolerance, 0, &mut points);
                current = Some(to);
            }
            PathSegment::CubicTo(c0, c1, point) => {
                let Some(from) = current else { continue };
                let c0 = point64(c0);
                let c1 = point64(c1);
                let to = point64(point);
                flatten_centerline(from, c0, c1, to, tolerance, 0, &mut points);
                current = Some(to);
            }
            PathSegment::Close => {
                closed = true;
                current = first;
            }
        }
    }
    flush(&mut result, &mut points, closed);
    result
}

fn collect_raw_subpaths(
    path: &SvgPath,
    tolerance: f64,
    transform: Affine64,
) -> Option<Vec<(Vec<Point64>, bool)>> {
    let mut result = Vec::new();
    let mut points = Vec::new();
    let mut first = None;
    let mut current = None;
    let mut closed = false;
    let flush =
        |result: &mut Vec<(Vec<Point64>, bool)>, points: &mut Vec<Point64>, closed: bool| {
            if points.len() >= 2 {
                result.push((std::mem::take(points), closed));
            } else {
                points.clear();
            }
        };
    let append = |points: &mut Vec<Point64>, point: Point64| {
        if points
            .last()
            .is_none_or(|previous| previous.x != point.x || previous.y != point.y)
        {
            points.push(point);
        }
    };
    let point64 = |point: SvgPoint| Point64 {
        x: point.x,
        y: point.y,
    };

    for command in &path.commands {
        match *command {
            SvgPathCommand::MoveTo(point) => {
                flush(&mut result, &mut points, closed);
                let point = point64(point);
                first = Some(point);
                current = Some(point);
                closed = false;
                append(&mut points, point);
            }
            SvgPathCommand::LineTo(point) => {
                let point = point64(point);
                append(&mut points, point);
                current = Some(point);
            }
            SvgPathCommand::QuadraticTo(control, point) => {
                let Some(from) = current else { continue };
                let point = point64(point);
                flatten_quadratic(from, point64(control), point, tolerance, 0, &mut points);
                current = Some(point);
            }
            SvgPathCommand::CubicTo(control0, control1, point) => {
                let Some(from) = current else { continue };
                let point = point64(point);
                flatten_centerline(
                    from,
                    point64(control0),
                    point64(control1),
                    point,
                    tolerance,
                    0,
                    &mut points,
                );
                current = Some(point);
            }
            SvgPathCommand::ArcTo {
                radius_x,
                radius_y,
                rotation,
                large_arc,
                sweep,
                end,
            } => {
                let Some(from) = current else { continue };
                let arc = SvgArc {
                    radius_x,
                    radius_y,
                    rotation,
                    large_arc,
                    sweep,
                    end: point64(end),
                };
                flatten_svg_arc(from, arc, transform, &mut points)?;
                current = Some(arc.end);
            }
            SvgPathCommand::Close => {
                closed = true;
                current = first;
            }
        }
    }
    flush(&mut result, &mut points, closed);
    Some(result)
}

#[derive(Clone, Copy)]
struct SvgArc {
    radius_x: f64,
    radius_y: f64,
    rotation: f64,
    large_arc: bool,
    sweep: bool,
    end: Point64,
}

#[derive(Clone, Copy)]
struct CenterArc {
    center: Point64,
    u: Point64,
    v: Point64,
    start: f64,
    delta: f64,
}

impl CenterArc {
    fn point_at(self, theta: f64) -> Point64 {
        let (sin, cos) = theta.sin_cos();
        Point64 {
            x: self.center.x + self.u.x * cos + self.v.x * sin,
            y: self.center.y + self.u.y * cos + self.v.y * sin,
        }
    }
}

/// Port of d2scene.EndpointArcToCenter. Keep the normalized radius ratios so
/// very small/large finite SVG radii follow the source numeric path too.
fn endpoint_arc_to_center(start: Point64, command: SvgArc) -> Option<CenterArc> {
    let end = command.end;
    if start == end {
        return None;
    }
    let mut radius_x = command.radius_x.abs();
    let mut radius_y = command.radius_y.abs();
    if radius_x == 0.0 || radius_y == 0.0 {
        return None;
    }
    let (sin_phi, cos_phi) = command.rotation.sin_cos();
    // Halve before subtracting as d2scene does, avoiding overflow at opposite
    // large finite endpoints.
    let dx = start.x / 2.0 - end.x / 2.0;
    let dy = start.y / 2.0 - end.y / 2.0;
    let x_prime = cos_phi * dx + sin_phi * dy;
    let y_prime = -sin_phi * dx + cos_phi * dy;
    if !x_prime.is_finite() || !y_prime.is_finite() {
        return None;
    }

    let (normalized_x, normalized_y, ratio_mantissa, ratio_exponent) =
        normalized_radius_ratios(x_prime, y_prime, radius_x, radius_y)?;
    let ratio_greater_or_equal_one =
        ratio_exponent > 0 || ratio_exponent == 0 && ratio_mantissa >= 1.0;
    let ratio_length = if ratio_greater_or_equal_one {
        radius_x = multiply_binary_scale(radius_x, ratio_mantissa, ratio_exponent);
        radius_y = multiply_binary_scale(radius_y, ratio_mantissa, ratio_exponent);
        if !radius_x.is_finite() || !radius_y.is_finite() {
            return None;
        }
        1.0
    } else {
        libm::ldexp(ratio_mantissa, ratio_exponent)
    };
    let mut root = (1.0 - ratio_length * ratio_length).max(0.0).sqrt();
    if command.large_arc == command.sweep {
        root = -root;
    }
    let cx_prime = (root * radius_x) * normalized_y;
    let cy_prime = (-root * radius_y) * normalized_x;
    let center = Point64 {
        x: cos_phi * cx_prime - sin_phi * cy_prime + start.x / 2.0 + end.x / 2.0,
        y: sin_phi * cx_prime + cos_phi * cy_prime + start.y / 2.0 + end.y / 2.0,
    };
    let u = Point64 {
        x: radius_x * cos_phi,
        y: radius_x * sin_phi,
    };
    let v = Point64 {
        x: -radius_y * sin_phi,
        y: radius_y * cos_phi,
    };
    if ![center.x, center.y, u.x, u.y, v.x, v.y]
        .into_iter()
        .all(f64::is_finite)
    {
        return None;
    }

    let (endpoint_x, endpoint_y) = if ratio_greater_or_equal_one {
        (normalized_x, normalized_y)
    } else {
        (normalized_x * ratio_length, normalized_y * ratio_length)
    };
    let cx_ratio = cx_prime / radius_x;
    let cy_ratio = cy_prime / radius_y;
    let ux = endpoint_x - cx_ratio;
    let uy = endpoint_y - cy_ratio;
    let vx = -endpoint_x - cx_ratio;
    let vy = -endpoint_y - cy_ratio;
    let start_angle = uy.atan2(ux);
    let mut delta = (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    if command.sweep && delta < 0.0 {
        delta += 2.0 * std::f64::consts::PI;
    } else if !command.sweep && delta > 0.0 {
        delta -= 2.0 * std::f64::consts::PI;
    }
    if !start_angle.is_finite() || !delta.is_finite() {
        return None;
    }
    Some(CenterArc {
        center,
        u: Point64 {
            x: radius_x * cos_phi,
            y: radius_x * sin_phi,
        },
        v: Point64 {
            x: -radius_y * sin_phi,
            y: radius_y * cos_phi,
        },
        start: start_angle,
        delta,
    })
}

fn normalized_radius_ratios(
    x_prime: f64,
    y_prime: f64,
    radius_x: f64,
    radius_y: f64,
) -> Option<(f64, f64, f64, i32)> {
    let ratio = |numerator: f64, denominator: f64| {
        if numerator == 0.0 {
            return None;
        }
        let (numerator_mantissa, numerator_exponent) = libm::frexp(numerator);
        let (denominator_mantissa, denominator_exponent) = libm::frexp(denominator);
        let (ratio_mantissa, ratio_exponent) =
            libm::frexp(numerator_mantissa / denominator_mantissa);
        Some((
            ratio_mantissa,
            numerator_exponent - denominator_exponent + ratio_exponent,
        ))
    };
    let x_ratio = ratio(x_prime, radius_x);
    let y_ratio = ratio(y_prime, radius_y);
    let max_exponent = match (x_ratio, y_ratio) {
        (None, None) => return None,
        (Some((_, exponent)), None) | (None, Some((_, exponent))) => exponent,
        (Some((_, x_exponent)), Some((_, y_exponent))) => x_exponent.max(y_exponent),
    };
    let scaled = |ratio: Option<(f64, i32)>| {
        ratio.map_or(0.0, |(mantissa, exponent)| {
            libm::ldexp(mantissa, exponent - max_exponent)
        })
    };
    let x_scaled = scaled(x_ratio);
    let y_scaled = scaled(y_ratio);
    let norm = x_scaled.hypot(y_scaled);
    if norm == 0.0 || !norm.is_finite() {
        return None;
    }
    let (ratio_mantissa, ratio_exponent) = libm::frexp(norm);
    Some((
        x_scaled / norm,
        y_scaled / norm,
        ratio_mantissa,
        max_exponent + ratio_exponent,
    ))
}

fn multiply_binary_scale(value: f64, mantissa: f64, exponent: i32) -> f64 {
    let (value_mantissa, value_exponent) = libm::frexp(value);
    libm::ldexp(value_mantissa * mantissa, value_exponent + exponent)
}

fn flatten_svg_arc(
    start: Point64,
    command: SvgArc,
    transform: Affine64,
    output: &mut Vec<Point64>,
) -> Option<()> {
    if start == command.end {
        return append_unique(output, command.end);
    }
    if command.radius_x == 0.0 || command.radius_y == 0.0 {
        return append_unique(output, command.end);
    }
    let arc = endpoint_arc_to_center(start, command)?;
    let transform_vector = |point: Point64| Point64 {
        x: transform.sx * point.x + transform.kx * point.y,
        y: transform.ky * point.x + transform.sy * point.y,
    };
    let u = transform_vector(arc.u);
    let v = transform_vector(arc.v);
    let radius_bound = u.x.hypot(u.y).hypot(v.x.hypot(v.y));
    if !radius_bound.is_finite() {
        return None;
    }
    flatten_svg_arc_interval(
        arc,
        arc.start,
        arc.start + arc.delta,
        command.end,
        radius_bound,
        0,
        output,
    )
}

fn flatten_svg_arc_interval(
    arc: CenterArc,
    theta0: f64,
    theta1: f64,
    end: Point64,
    radius_bound: f64,
    depth: usize,
    output: &mut Vec<Point64>,
) -> Option<()> {
    let span = (theta1 - theta0).abs();
    let deviation_factor = span * span / 8.0;
    if deviation_factor == 0.0 || radius_bound <= 0.25 / deviation_factor {
        return append_unique(output, end);
    }
    if depth >= MAX_CURVE_DEPTH {
        return None;
    }
    let middle_theta = theta0 + (theta1 - theta0) / 2.0;
    let middle = arc.point_at(middle_theta);
    if !middle.x.is_finite() || !middle.y.is_finite() {
        return None;
    }
    flatten_svg_arc_interval(
        arc,
        theta0,
        middle_theta,
        middle,
        radius_bound,
        depth + 1,
        output,
    )?;
    flatten_svg_arc_interval(
        arc,
        middle_theta,
        theta1,
        end,
        radius_bound,
        depth + 1,
        output,
    )
}

fn append_unique(output: &mut Vec<Point64>, point: Point64) -> Option<()> {
    if !point.x.is_finite() || !point.y.is_finite() {
        return None;
    }
    if output.last().is_none_or(|previous| *previous != point) {
        output.push(point);
    }
    Some(())
}

fn collect_fixed_subpaths(path: &FixedGlyphPath, tolerance: f64) -> Vec<(Vec<Point64>, bool)> {
    let mut result = Vec::new();
    let mut points = Vec::new();
    let mut first = None;
    let mut current = None;
    let mut closed = false;
    let flush =
        |result: &mut Vec<(Vec<Point64>, bool)>, points: &mut Vec<Point64>, closed: bool| {
            if points.len() >= 2 {
                result.push((std::mem::take(points), closed));
            } else {
                points.clear();
            }
        };
    let append = |points: &mut Vec<Point64>, point: Point64| {
        if points
            .last()
            .is_none_or(|previous| previous.x != point.x || previous.y != point.y)
        {
            points.push(point);
        }
    };
    for segment in &path.segments {
        match *segment {
            FixedPathSegment::MoveTo(point) => {
                flush(&mut result, &mut points, closed);
                first = Some(point);
                current = Some(point);
                closed = false;
                append(&mut points, point);
            }
            FixedPathSegment::LineTo(point) => {
                append(&mut points, point);
                current = Some(point);
            }
            FixedPathSegment::QuadTo(control, point) => {
                let Some(from) = current else { continue };
                flatten_quadratic(from, control, point, tolerance, 0, &mut points);
                current = Some(point);
            }
            FixedPathSegment::CubicTo(control0, control1, point) => {
                let Some(from) = current else { continue };
                flatten_centerline_scene(
                    from,
                    control0,
                    control1,
                    point,
                    tolerance,
                    0,
                    &mut points,
                );
                current = Some(point);
            }
            FixedPathSegment::Close => {
                closed = true;
                current = first;
            }
        }
    }
    flush(&mut result, &mut points, closed);
    result
}

fn point_line_distance(point: Point64, start: Point64, end: Point64) -> f64 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length = dx.hypot(dy);
    if length == 0.0 {
        return (point.x - start.x).hypot(point.y - start.y);
    }
    // ARM64 geometry.go:382: the subtracted product is fused.
    d2_rs_render::go_float::go_fmsub(start.x - point.x, dy, dx * (start.y - point.y)).abs() / length
}

fn controls_progress(start: Point64, end: Point64, tolerance: f64, controls: &[Point64]) -> bool {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length = dx.hypot(dy);
    if length == 0.0 {
        return controls
            .iter()
            .all(|control| (control.x - start.x).hypot(control.y - start.y) <= tolerance);
    }
    let unit_x = dx / length;
    let unit_y = dy / length;
    let mut previous = 0.0;
    for control in controls {
        let control_x = control.x - start.x;
        let control_y = control.y - start.y;
        // ARM64 geometry.go:400–401: round the first product, then fuse
        // the second. A one-ulp change at tolerance must subdivide the curve.
        let projection = d2_rs_render::go_float::go_fmadd(control_y, unit_y, control_x * unit_x);
        let perpendicular =
            d2_rs_render::go_float::go_fmsub(control_y, unit_x, control_x * unit_y).abs();
        let excess = if projection < 0.0 {
            -projection
        } else if projection > length {
            projection - length
        } else {
            0.0
        };
        if !projection.is_finite()
            || !perpendicular.is_finite()
            || perpendicular.hypot(excess) > tolerance
            || projection < previous - tolerance
        {
            return false;
        }
        previous = projection;
    }
    true
}

fn flatten_quadratic(
    p0: Point64,
    p1: Point64,
    p2: Point64,
    tolerance: f64,
    depth: usize,
    output: &mut Vec<Point64>,
) {
    if point_line_distance(p1, p0, p2) <= tolerance && controls_progress(p0, p2, tolerance, &[p1]) {
        output.push(p2);
        return;
    }
    if depth >= MAX_CURVE_DEPTH {
        output.push(p2);
        return;
    }
    let p01 = midpoint_scene(p0, p1);
    let p12 = midpoint_scene(p1, p2);
    let middle = midpoint_scene(p01, p12);
    flatten_quadratic(p0, p01, middle, tolerance, depth + 1, output);
    flatten_quadratic(middle, p12, p2, tolerance, depth + 1, output);
}

fn collect_ellipse_centerline(
    path: &Path,
    _transform: Transform,
    tolerance: f64,
) -> Option<(Vec<Point64>, bool)> {
    let segments = path.segments().collect::<Vec<_>>();
    let (Some(PathSegment::MoveTo(start)), Some(PathSegment::Close)) =
        (segments.first(), segments.last())
    else {
        return None;
    };
    let curves = &segments[1..segments.len() - 1];
    if !matches!(curves.len(), 4 | 8)
        || !curves
            .iter()
            .all(|segment| matches!(segment, PathSegment::CubicTo(..)))
    {
        return None;
    }
    let endpoint = |index: usize| match curves[index] {
        PathSegment::CubicTo(_, _, point) => Some(point),
        _ => None,
    };
    // SVG arc lowering may use one or two cubics per quadrant. Recover the
    // four axis extrema either way; d2raster builds those four f64 cubics
    // directly from the scene ellipse before flattening.
    let quarter = curves.len() / 4;
    let first_end = endpoint(quarter - 1)?;
    let second_end = endpoint(quarter * 2 - 1)?;
    let third_end = endpoint(quarter * 3 - 1)?;
    let fourth_end = endpoint(quarter * 4 - 1)?;
    // D2 reconstructs the ellipse from f64 scene coordinates, flattens it in
    // scene space using the transform-adjusted tolerance, then applies the
    // output transform. Keep that order here instead of flattening usvg's
    // already-transformed f32 path; the difference is visible at circle
    // boundaries where one subdivision lands on a pixel edge.
    let start = Point64 {
        x: f64::from(start.x),
        y: f64::from(start.y),
    };
    let first_end = Point64 {
        x: f64::from(first_end.x),
        y: f64::from(first_end.y),
    };
    let second_end = Point64 {
        x: f64::from(second_end.x),
        y: f64::from(second_end.y),
    };
    let third_end = Point64 {
        x: f64::from(third_end.x),
        y: f64::from(third_end.y),
    };
    let fourth_end = Point64 {
        x: f64::from(fourth_end.x),
        y: f64::from(fourth_end.y),
    };
    if (fourth_end.x - start.x).abs() > f64::EPSILON
        || (fourth_end.y - start.y).abs() > f64::EPSILON
        || (start.x - second_end.x).abs() <= f64::EPSILON
        || (first_end.y - third_end.y).abs() <= f64::EPSILON
    {
        return None;
    }
    let center = Point64 {
        x: (start.x + second_end.x) * 0.5,
        y: (first_end.y + third_end.y) * 0.5,
    };
    let radius_x = (start.x - second_end.x).abs() * 0.5;
    let radius_y = (first_end.y - third_end.y).abs() * 0.5;
    let k = 0.552_284_749_830_793_6;
    let curves = [
        (
            start,
            Point64 {
                x: center.x + radius_x,
                y: center.y + k * radius_y,
            },
            Point64 {
                x: center.x + k * radius_x,
                y: center.y + radius_y,
            },
            first_end,
        ),
        (
            first_end,
            Point64 {
                x: center.x - k * radius_x,
                y: center.y + radius_y,
            },
            Point64 {
                x: center.x - radius_x,
                y: center.y + k * radius_y,
            },
            second_end,
        ),
        (
            second_end,
            Point64 {
                x: center.x - radius_x,
                y: center.y - k * radius_y,
            },
            Point64 {
                x: center.x - k * radius_x,
                y: center.y - radius_y,
            },
            third_end,
        ),
        (
            third_end,
            Point64 {
                x: center.x + k * radius_x,
                y: center.y - radius_y,
            },
            Point64 {
                x: center.x + radius_x,
                y: center.y - k * radius_y,
            },
            fourth_end,
        ),
    ];
    let mut source_points = vec![start];
    for (p0, p1, p2, p3) in curves {
        flatten_centerline(p0, p1, p2, p3, tolerance, 0, &mut source_points);
    }
    if source_points.len() > 1
        && source_points
            .last()
            .is_some_and(|point| point.x == start.x && point.y == start.y)
    {
        source_points.pop();
    }
    Some((source_points, true))
}

fn map_centerline(centerline: (Vec<Point64>, bool), transform: Affine64) -> (Vec<Point64>, bool) {
    let (points, closed) = centerline;
    let points = points
        .into_iter()
        .map(|point| Point64 {
            // Keep this multiplication in f64. Go's scene transform is f64
            // and the scanline API performs the float32 rounding only when a
            // point is submitted, whereas tiny-skia maps a path in f32.
            x: point.x * transform.sx + point.y * transform.kx + transform.tx,
            y: point.x * transform.ky + point.y * transform.sy + transform.ty,
        })
        .collect();
    (points, closed)
}

fn flatten_tolerance(transform: Transform) -> f64 {
    let a = f64::from(transform.sx);
    let b = f64::from(transform.kx);
    let c = f64::from(transform.ky);
    let d = f64::from(transform.sy);
    let max_coefficient = a.abs().max(b.abs()).max(c.abs()).max(d.abs());
    if !max_coefficient.is_finite() {
        return f64::MIN_POSITIVE;
    }
    if max_coefficient == 0.0 {
        return f64::MAX;
    }
    let na = a / max_coefficient;
    let nb = b / max_coefficient;
    let nc = c / max_coefficient;
    let nd = d / max_coefficient;
    let trace = na * na + nb * nb + nc * nc + nd * nd;
    let determinant = na * nd - nb * nc;
    let discriminant = (trace * trace - 4.0 * determinant * determinant).max(0.0);
    let factor = ((trace + discriminant.sqrt()) * 0.5).sqrt();
    if factor == 0.0 {
        return f64::MAX;
    }
    0.25 / (max_coefficient * factor)
}

fn unit_vector(start: Point64, end: Point64) -> Option<Point64> {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length = (dx * dx + dy * dy).sqrt();
    (length.is_finite() && length > 0.0).then_some(Point64 {
        x: dx / length,
        y: dy / length,
    })
}

fn flatten_centerline(
    p0: Point64,
    p1: Point64,
    p2: Point64,
    p3: Point64,
    tolerance: f64,
    depth: usize,
    output: &mut Vec<Point64>,
) {
    if cube_flat_enough(p0, p1, p2, p3, tolerance) || depth >= MAX_CURVE_DEPTH {
        output.push(p3);
        return;
    }
    // D2's scene flattener averages the two control coordinates directly.
    // Keep this separate from the scanline kernel's difference-form midpoint,
    // whose last-bit behavior is part of its reference-rounded API.
    let p01 = midpoint_scene(p0, p1);
    let p12 = midpoint_scene(p1, p2);
    let p23 = midpoint_scene(p2, p3);
    let p012 = midpoint_scene(p01, p12);
    let p123 = midpoint_scene(p12, p23);
    let middle = midpoint_scene(p012, p123);
    flatten_centerline(p0, p01, p012, middle, tolerance, depth + 1, output);
    flatten_centerline(middle, p123, p23, p3, tolerance, depth + 1, output);
}

fn flatten_centerline_scene(
    p0: Point64,
    p1: Point64,
    p2: Point64,
    p3: Point64,
    tolerance: f64,
    depth: usize,
    output: &mut Vec<Point64>,
) {
    if cube_flat_enough(p0, p1, p2, p3, tolerance) || depth >= MAX_CURVE_DEPTH {
        output.push(p3);
        return;
    }
    let p01 = midpoint_scene(p0, p1);
    let p12 = midpoint_scene(p1, p2);
    let p23 = midpoint_scene(p2, p3);
    let p012 = midpoint_scene(p01, p12);
    let p123 = midpoint_scene(p12, p23);
    let middle = midpoint_scene(p012, p123);
    flatten_centerline_scene(p0, p01, p012, middle, tolerance, depth + 1, output);
    flatten_centerline_scene(middle, p123, p23, p3, tolerance, depth + 1, output);
}

fn coverage(area: f32) -> u8 {
    let area = area.abs();
    if area <= 0.0 {
        0
    } else if area >= 1.0 {
        255
    } else {
        (area * 255.0 + 0.5) as u8
    }
}

// Source zeroCoverageValues/zeroCoverageRunEnd preserves +/-zero equality;
// no epsilon or coverage approximation is admitted into span discovery.
fn zero_coverage_values(partial: f32, difference: f32) -> bool {
    (partial.to_bits() | difference.to_bits()) << 1 == 0
}

fn zero_coverage_run_end(partial: &[f32], difference: &[f32], mut x: usize, end: usize) -> usize {
    while x + 4 <= end {
        let bits = partial[x].to_bits()
            | difference[x].to_bits()
            | partial[x + 1].to_bits()
            | difference[x + 1].to_bits()
            | partial[x + 2].to_bits()
            | difference[x + 2].to_bits()
            | partial[x + 3].to_bits()
            | difference[x + 3].to_bits();
        if bits << 1 != 0 {
            break;
        }
        x += 4;
    }
    while x < end && zero_coverage_values(partial[x], difference[x]) {
        x += 1;
    }
    x
}

fn paint_solid_span(pixels: &mut [u8], color: [u8; 4], coverage: u8) {
    if pixels.is_empty() || color[3] == 0 || coverage == 0 {
        return;
    }
    if color[3] == 255 && coverage == 255 {
        // Original fillPackedRGBA: grow an initialized packed-color prefix.
        pixels[..4].copy_from_slice(&color);
        let mut filled = 4;
        while filled < pixels.len() {
            let count = filled.min(pixels.len() - filled);
            pixels.copy_within(..count, filled);
            filled += count;
        }
        return;
    }
    let mask = u64::from(coverage) * 257;
    let alpha = u64::from(color[3]) * 257 * mask / 65_535;
    let inverse = (65_535 - alpha) * 257;
    let products: [u64; 3] =
        std::array::from_fn(|c| u64::from(color[c]) * 257 * u64::from(color[3]) / 255 * mask);
    for pixel in pixels.chunks_exact_mut(4) {
        for c in 0..3 {
            pixel[c] = (((u64::from(pixel[c]) * inverse + products[c]) / 65_535) >> 8) as u8;
        }
        pixel[3] = (((u64::from(pixel[3]) * inverse + 65_535 * alpha) / 65_535) >> 8) as u8;
    }
}

fn blend_pixel(pixel: &mut [u8], color: [u8; 4], coverage: u8) {
    // Source DrawRGBA's sparse and dense opaque paths both replace a fully
    // covered pixel directly. Keep partial coverage on the wide-alpha path.
    if color[3] == 0xff && coverage == 0xff {
        pixel.copy_from_slice(&color);
        return;
    }
    // Keep the replicated-alpha arithmetic in 64 bits. Low-coverage pixels
    // can otherwise overflow the channel product before the final shift.
    let mask = u64::from(coverage);
    let mask16 = mask | (mask << 8);
    let source_alpha = u64::from(color[3]) * 257;
    let effective_alpha = source_alpha * mask16 / 65_535;
    let inverse = (65_535 - effective_alpha) * 257;
    for channel in 0..3 {
        // Original Go color.NRGBA.RGBA premultiplies the 16-bit color
        // channels before the scanline DrawRGBA mask arithmetic.
        let premultiplied16 = u64::from(color[channel]) * 257 * u64::from(color[3]) / 255;
        pixel[channel] = (((u64::from(pixel[channel]) * inverse + premultiplied16 * mask16)
            / 65_535)
            >> 8) as u8;
    }
    pixel[3] = (((u64::from(pixel[3]) * inverse + 65_535 * effective_alpha) / 65_535) >> 8) as u8;
}

/// d2raster's byte-domain NRGBA-over-RGBA compositor, retained for imported
/// SVGs whose path paint remains in the document canvas. The ordinary scene
/// path keeps the historical wide-alpha kernel above; using the source
/// compositor here isolates the imported-vector parity boundary.
fn blend_pixel_go(pixel: &mut [u8], color: [u8; 4], coverage: u8) {
    if color[3] == 0xff && coverage == 0xff {
        pixel[..3].copy_from_slice(&color[..3]);
        pixel[3] = 0xff;
        return;
    }
    let mul255 = |a: u32, b: u32| (a * b + 127) / 255;
    let source_alpha = mul255(u32::from(color[3]), u32::from(coverage));
    if pixel.iter().all(|value| *value == 0) {
        for channel in 0..3 {
            pixel[channel] = mul255(u32::from(color[channel]), source_alpha) as u8;
        }
        pixel[3] = source_alpha as u8;
        return;
    }
    let inverse_alpha = 255 - source_alpha;
    for channel in 0..3 {
        let premultiplied = mul255(u32::from(color[channel]), source_alpha);
        pixel[channel] = premultiplied
            .saturating_add(mul255(u32::from(pixel[channel]), inverse_alpha))
            .min(255) as u8;
    }
    pixel[3] = source_alpha
        .saturating_add(mul255(u32::from(pixel[3]), inverse_alpha))
        .min(255) as u8;
}

fn blend_premultiplied_pixel(pixel: &mut [u8], source: [u8; 4]) {
    // Original effects.go compositeLayerOver skips transparent sources and
    // copies opaque sources. Avoid touching the destination for empty mask/
    // effect-layer pixels, which commonly cover most of a large canvas.
    if source[3] == 0 {
        return;
    }
    if source[3] == 255 {
        pixel.copy_from_slice(&source);
        return;
    }
    // Go's retained renderer composites a full-opacity RGBA effect layer with
    // byte-domain source-over arithmetic. The layer RGB bytes are already
    // premultiplied, so preserve that representation and round each 8-bit
    // multiply exactly as its mul255 helper does.
    let source_alpha = u32::from(source[3]);
    let inverse = 255_u32 - source_alpha;
    for channel in 0..3 {
        let value = u32::from(source[channel]) + (u32::from(pixel[channel]) * inverse + 127) / 255;
        pixel[channel] = value.min(255) as u8;
    }
    pixel[3] = (source_alpha + (u32::from(pixel[3]) * inverse + 127) / 255).min(255) as u8;
}

#[cfg(feature = "diagnostic-traces")]
fn raster_trace_pixel() -> Option<(usize, usize)> {
    let raw = std::env::var("D2_RS_TRACE_RASTER_PIXEL").ok()?;
    let (x, y) = raw.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

pub(super) fn solid_paint(paint: &Paint, opacity: f32) -> Option<[u8; 4]> {
    let Paint::Color(color) = paint else {
        return None;
    };
    // Original d2svgimport/style.go:paintWithOpacity rounds before raster coverage.
    let alpha = (255.0 * opacity).round().clamp(0.0, 255.0) as u8;
    Some([color.red, color.green, color.blue, alpha])
}

pub(super) fn solid_paint_exact(
    paint: &Paint,
    opacity: f32,
    exact: Option<f64>,
) -> Option<[u8; 4]> {
    let mut color = solid_paint(paint, opacity)?;
    if let Some(value) = exact.filter(|value| (*value as f32).to_bits() == opacity.to_bits()) {
        color[3] = (255.0 * value).round().clamp(0.0, 255.0) as u8;
    }
    Some(color)
}

fn paint_color_at(
    paint: &Paint,
    opacity: f32,
    device_transform: Transform,
    x: usize,
    y: usize,
    pattern_tiles: &PatternInstances,
    exact_transform: Option<Affine64>,
    prepared_pattern_transform: Option<Affine64>,
    device_origin: Point64,
    raw_stops: &HashMap<String, crate::svg_precision::RawGradient>,
) -> Option<[u8; 4]> {
    let Paint::Pattern(pattern) = paint else {
        return match paint {
            Paint::LinearGradient(gradient) => linear_gradient_color_at(
                gradient,
                opacity,
                device_transform,
                x,
                y,
                raw_stops.get(gradient.id()),
            ),
            Paint::RadialGradient(gradient) => radial_gradient_color_at(
                gradient,
                opacity,
                device_transform,
                x,
                y,
                raw_stops.get(gradient.id()),
            ),
            _ => solid_paint(paint, opacity),
        };
    };
    pattern::sampler(
        pattern,
        opacity,
        device_transform,
        exact_transform,
        device_origin,
        prepared_pattern_transform,
        pattern_tiles,
    )?
    .color_at(x, y)
}

/// `paint_color_at` with pattern tiles and gradient coordinate mappings
/// resolved once for the draw instead of once per covered pixel.
#[allow(clippy::too_many_arguments)]
fn paint_sampler<'a>(
    paint: &'a Paint,
    opacity: f32,
    device_transform: Transform,
    pattern_tiles: &'a PatternInstances,
    exact_transform: Option<Affine64>,
    prepared_pattern_transform: Option<Affine64>,
    device_origin: Point64,
    raw_stops: &'a HashMap<String, crate::svg_precision::RawGradient>,
) -> impl Fn(usize, usize) -> Option<[u8; 4]> + 'a {
    let pattern = match paint {
        Paint::Pattern(p) => Some(pattern::sampler(
            p,
            opacity,
            device_transform,
            exact_transform,
            device_origin,
            prepared_pattern_transform,
            pattern_tiles,
        )),
        _ => None,
    };
    // A gradient's document stops, looked up once rather than per pixel.
    let stops = match paint {
        Paint::LinearGradient(gradient) => raw_stops.get(gradient.id()),
        Paint::RadialGradient(gradient) => raw_stops.get(gradient.id()),
        _ => None,
    };
    let linear = match paint {
        Paint::LinearGradient(gradient) => Some(linear_gradient_sampler(
            gradient,
            opacity,
            stops
                .and_then(|raw| raw.native_transform)
                .and(exact_transform)
                .unwrap_or_else(|| glyph_transform64(device_transform)),
            Point64::default(),
            stops,
        )),
        _ => None,
    };
    move |x, y| match (&pattern, paint) {
        (Some(sampler), _) => sampler.as_ref()?.color_at(x, y),
        (None, Paint::LinearGradient(_)) => linear.as_ref()?(x, y),
        (None, Paint::RadialGradient(gradient)) => radial_gradient_color_at64(
            gradient,
            opacity,
            stops
                .and_then(|raw| raw.native_transform)
                .and(exact_transform)
                .unwrap_or_else(|| glyph_transform64(device_transform)),
            x,
            y,
            stops,
        ),
        (None, _) => paint_color_at(
            paint,
            opacity,
            device_transform,
            x,
            y,
            pattern_tiles,
            exact_transform,
            prepared_pattern_transform,
            device_origin,
            raw_stops,
        ),
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
fn gradient_inverse(transform: Transform, device_transform: Transform) -> Option<Affine64> {
    // Source prepares objectToDevice * unitsToObject * gradientTransform.
    // usvg folds the last two together; preserve matrix order and f64
    // sampling instead of projecting into the nonuniform device space.
    affine_inverse(affine_mul(
        glyph_transform64(device_transform),
        glyph_transform64(transform),
    ))
}
fn spread_gradient_parameter(value: f64, spread: usvg::SpreadMethod) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    Some(match spread {
        usvg::SpreadMethod::Pad => value.clamp(0.0, 1.0),
        usvg::SpreadMethod::Reflect => {
            let mut value = value % 2.0;
            if value < 0.0 {
                value += 2.0;
            }
            if value > 1.0 {
                value = 2.0 - value;
            }
            value
        }
        usvg::SpreadMethod::Repeat => value - value.floor(),
    })
}

fn linear_gradient_color_at(
    gradient: &usvg::LinearGradient,
    opacity: f32,
    device_transform: Transform,
    x: usize,
    y: usize,
    raw_stops: Option<&crate::svg_precision::RawGradient>,
) -> Option<[u8; 4]> {
    linear_gradient_color_at64(
        gradient,
        opacity,
        glyph_transform64(device_transform),
        x,
        y,
        Point64 { x: 0.0, y: 0.0 },
        raw_stops,
    )
}

fn linear_gradient_color_at64(
    gradient: &usvg::LinearGradient,
    opacity: f32,
    device_transform: Affine64,
    x: usize,
    y: usize,
    device_origin: Point64,
    raw_stops: Option<&crate::svg_precision::RawGradient>,
) -> Option<[u8; 4]> {
    linear_gradient_sampler(
        gradient,
        opacity,
        device_transform,
        device_origin,
        raw_stops,
    )(x, y)
}

/// Source gradient.go prepareLinearGradient/prepareGradientTransform keeps
/// the coordinate mapping with the paint. Prepare it once for a draw, without
/// changing colorAt's fused operations or its per-pixel evaluation order.
fn linear_gradient_sampler<'a>(
    gradient: &'a usvg::LinearGradient,
    opacity: f32,
    device_transform: Affine64,
    device_origin: Point64,
    raw_stops: Option<&'a crate::svg_precision::RawGradient>,
) -> impl Fn(usize, usize) -> Option<[u8; 4]> + 'a {
    // usvg folds objectBoundingBox into the transform and keeps the vector
    // in f32; retain the document's f64 vector when it was parsed.
    let [x1, y1, x2, y2] = raw_stops.and_then(|raw| raw.vector).unwrap_or([
        f64::from(gradient.x1()),
        f64::from(gradient.y1()),
        f64::from(gradient.x2()),
        f64::from(gradient.y2()),
    ]);
    let start = Point64 { x: x1, y: y1 };
    let delta = Point64 {
        x: x2 - start.x,
        y: y2 - start.y,
    };
    let denominator = d2_rs_render::go_float::go_fmadd(delta.x, delta.x, delta.y * delta.y);
    let constant = denominator == 0.0 || gradient.stops().len() == 1;
    let inverse = (!constant)
        .then(|| {
            affine_inverse(source_affine_mul(
                device_transform,
                raw_stops
                    .and_then(|raw| raw.native_transform)
                    .map(|m| Affine64 {
                        sx: m[0],
                        ky: m[1],
                        kx: m[2],
                        sy: m[3],
                        tx: m[4],
                        ty: m[5],
                    })
                    .unwrap_or_else(|| glyph_transform64(gradient.transform())),
            ))
        })
        .flatten();
    // gradient.go:drawLinearYOnlyGradientCoverage samples once per row.
    // Keep the same coordinate arithmetic and share its result across that
    // row's coverage samples, including transparent samples.
    let row_only = delta.x == 0.0 && inverse.is_some_and(|inverse| inverse.ky == 0.0);
    let row_sample = std::cell::Cell::new(None::<(usize, Option<[u8; 4]>)>);
    move |x, y| {
        if row_only
            && let Some((row, sample)) = row_sample.get()
            && row == y
        {
            return sample;
        }
        if constant {
            return gradient_color_with_raw(gradient.stops(), raw_stops, 1.0, opacity);
        }
        let inverse = inverse?;
        let row = y;
        let (x, y) = (
            x as f64 + 0.5 + device_origin.x,
            y as f64 + 0.5 + device_origin.y,
        );
        // Go 1.27 ARM64 colorAt has distinct fused axis expressions. The
        // coverage and mask painters that gradient fills and strokes reach
        // evaluate x-only gradients as xC*x + 0*y + offset, so the product is
        // rounded before the offset is added.
        let parameter = if delta.y == 0.0 {
            let coordinate = if inverse.kx == 0.0 {
                inverse.sx * x + inverse.tx
            } else {
                d2_rs_render::go_float::go_fmadd(inverse.sx, x, inverse.kx * y) + inverse.tx
            };
            (coordinate - start.x) * delta.x / denominator
        } else if delta.x == 0.0 {
            let coordinate = if inverse.ky == 0.0 {
                d2_rs_render::go_float::go_fmadd(inverse.sy, y, inverse.ty)
            } else {
                d2_rs_render::go_float::go_fmadd(inverse.sy, y, inverse.ky * x) + inverse.ty
            };
            (coordinate - start.y) * delta.y / denominator
        } else {
            let point = Point64 {
                x: d2_rs_render::go_float::go_fmadd(inverse.sx, x, inverse.kx * y) + inverse.tx,
                y: d2_rs_render::go_float::go_fmadd(inverse.ky, x, inverse.sy * y) + inverse.ty,
            };
            d2_rs_render::go_float::go_fmadd(
                point.x - start.x,
                delta.x,
                (point.y - start.y) * delta.y,
            ) / denominator
        };
        let sample = gradient_color_with_raw(
            gradient.stops(),
            raw_stops,
            spread_gradient_parameter(parameter, gradient.spread_method())?,
            opacity,
        );
        if row_only {
            row_sample.set(Some((row, sample)));
        }
        sample
    }
}

fn radial_gradient_color_at(
    gradient: &usvg::RadialGradient,
    opacity: f32,
    device_transform: Transform,
    x: usize,
    y: usize,
    raw_stops: Option<&crate::svg_precision::RawGradient>,
) -> Option<[u8; 4]> {
    radial_gradient_color_at64(
        gradient,
        opacity,
        glyph_transform64(device_transform),
        x,
        y,
        raw_stops,
    )
}

fn radial_gradient_color_at64(
    gradient: &usvg::RadialGradient,
    opacity: f32,
    device_transform: Affine64,
    x: usize,
    y: usize,
    raw_stops: Option<&crate::svg_precision::RawGradient>,
) -> Option<[u8; 4]> {
    if gradient.stops().len() == 1 {
        return gradient_color_with_raw(gradient.stops(), raw_stops, 1.0, opacity);
    }
    let local = raw_stops
        .and_then(|raw| raw.native_transform)
        .map(|m| Affine64 {
            sx: m[0],
            ky: m[1],
            kx: m[2],
            sy: m[3],
            tx: m[4],
            ty: m[5],
        })
        .unwrap_or_else(|| glyph_transform64(gradient.transform()));
    let inverse = affine_inverse(
        if raw_stops.is_some_and(|raw| raw.native_transform.is_some()) {
            source_affine_mul(device_transform, local)
        } else {
            affine_mul(device_transform, local)
        },
    )?;
    let (x, y) = (x as f64 + 0.5, y as f64 + 0.5);
    let point = Point64 {
        x: d2_rs_render::go_float::go_fmadd(inverse.sx, x, inverse.kx * y) + inverse.tx,
        y: d2_rs_render::go_float::go_fmadd(inverse.ky, x, inverse.sy * y) + inverse.ty,
    };
    let circles = [
        f64::from(gradient.fx()),
        f64::from(gradient.fy()),
        f64::from(gradient.fr().get()),
        f64::from(gradient.cx()),
        f64::from(gradient.cy()),
        f64::from(gradient.r().get()),
    ];
    let parameter = colr_paint::radial(circles, point)?;
    gradient_color_with_raw(
        gradient.stops(),
        raw_stops,
        spread_gradient_parameter(parameter, gradient.spread_method())?,
        opacity,
    )
}

/// `origin % period` for `wrapped_pattern_remainder`, or None when the tile
/// axis cannot wrap any coordinate.
pub(super) fn pattern_origin_remainder(origin: f64, period: f64) -> Option<f64> {
    if !origin.is_finite() || !period.is_finite() || period <= 0.0 {
        return None;
    }
    Some(origin % period)
}

pub(super) fn wrapped_pattern_remainder(
    value: f64,
    origin_remainder: f64,
    period: f64,
) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    // Original math.Mod retains a tiny negative remainder until after
    // origin subtraction. rem_euclid can round it to the period early,
    // changing the selected edge pixel.
    let mut wrapped = value % period - origin_remainder;
    if !wrapped.is_finite() {
        return None;
    }
    if wrapped >= period {
        wrapped -= period;
    } else if wrapped < 0.0 {
        if wrapped == -period {
            return Some(-0.0);
        }
        wrapped += period;
        if wrapped < 0.0 {
            wrapped += period;
        }
    }
    Some(wrapped)
}

fn gradient_color_with_raw(
    stops: &[usvg::Stop],
    raw: Option<&crate::svg_precision::RawGradient>,
    parameter: f64,
    opacity: f32,
) -> Option<[u8; 4]> {
    let Some(stops) = raw.map(|raw| raw.stops.as_slice()) else {
        return gradient_color(stops, parameter, opacity);
    };
    let first = stops.first()?;
    let last = stops.last()?;
    let (left, right) = if parameter < first.offset {
        (first, first)
    } else if parameter >= last.offset {
        (last, last)
    } else {
        stops
            .windows(2)
            .find(|s| parameter < s[1].offset)
            .map(|s| (&s[0], &s[1]))?
    };
    let amount = if std::ptr::eq(left, right) {
        0.0
    } else {
        (parameter - left.offset) / (right.offset - left.offset)
    };
    let mut color = std::array::from_fn(|i| {
        d2_rs_render::go_float::go_fmadd(
            f64::from(right.color[i]) - f64::from(left.color[i]),
            amount,
            f64::from(left.color[i]),
        )
        .round() as u8
    });
    color[3] = (f64::from(color[3]) * f64::from(opacity)).clamp(0.0, 255.0) as u8;
    Some(color)
}

fn gradient_color(stops: &[usvg::Stop], parameter: f64, opacity: f32) -> Option<[u8; 4]> {
    let first = stops.first()?;
    let last = stops.last()?;
    let (left, right) = if stops.len() == 1 || parameter < f64::from(first.offset().get()) {
        (first, first)
    } else if parameter >= f64::from(last.offset().get()) {
        (last, last)
    } else {
        stops
            .windows(2)
            .find(|pair| parameter < f64::from(pair[1].offset().get()))
            .map(|pair| (&pair[0], &pair[1]))?
    };
    let amount = if std::ptr::eq(left, right) {
        0.0
    } else {
        let denominator = f64::from(right.offset().get()) - f64::from(left.offset().get());
        if denominator == 0.0 {
            0.0
        } else {
            (parameter - f64::from(left.offset().get())) / denominator
        }
    };
    let lerp = |a: u8, b: u8| -> u8 {
        d2_rs_render::go_float::go_fmadd(f64::from(b) - f64::from(a), amount, f64::from(a)).round()
            as u8
    };
    let alpha = f64::from(lerp(
        (f64::from(left.opacity().get()) * 255.0).round() as u8,
        (f64::from(right.opacity().get()) * 255.0).round() as u8,
    )) * f64::from(opacity);
    Some([
        lerp(left.color().red, right.color().red),
        lerp(left.color().green, right.color().green),
        lerp(left.color().blue, right.color().blue),
        alpha.clamp(0.0, 255.0) as u8,
    ])
}

/// Source effects.go rasterizeEvenOddMask/countEvenOddPathSamples. Four by
/// four samples share each strict edge crossing across their horizontal row.
fn paint_even_odd_paths<F>(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    paths: &[(Vec<Point64>, bool)],
    mut color_at: F,
) where
    F: FnMut(usize, usize) -> Option<[u8; 4]>,
{
    let mut left = f64::INFINITY;
    let mut top = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    let mut bottom = f64::NEG_INFINITY;
    for (points, _) in paths {
        for point in points {
            left = left.min(point.x);
            top = top.min(point.y);
            right = right.max(point.x);
            bottom = bottom.max(point.y);
        }
    }
    let x0 = left.floor().clamp(0.0, width as f64) as usize;
    let x1 = right.ceil().clamp(0.0, width as f64) as usize;
    let y0 = top.floor().clamp(0.0, height as f64) as usize;
    let y1 = bottom.ceil().clamp(0.0, height as f64) as usize;
    for y in y0..y1 {
        for x in x0..x1 {
            let mut inside = 0;
            for sy in [0.125, 0.375, 0.625, 0.875] {
                let sample_y = y as f64 + sy;
                let mut parity = 0_u8;
                for (points, _) in paths {
                    if points.len() < 2 {
                        continue;
                    }
                    let mut previous = *points.last().unwrap();
                    for current in points {
                        if (current.y > sample_y) != (previous.y > sample_y) {
                            let crossing = (previous.x - current.x) * (sample_y - current.y)
                                / (previous.y - current.y)
                                + current.x;
                            parity ^= if (x as f64 + 0.875) < crossing {
                                0b1111
                            } else if (x as f64 + 0.625) < crossing {
                                0b0111
                            } else if (x as f64 + 0.375) < crossing {
                                0b0011
                            } else if (x as f64 + 0.125) < crossing {
                                0b0001
                            } else {
                                0
                            };
                        }
                        previous = *current;
                    }
                }
                inside += parity.count_ones();
            }
            let coverage = ((inside * 255 + 8) / 16) as u8;
            if coverage != 0
                && let Some(color) = color_at(x, y)
            {
                let offset = (y * width + x) * 4;
                blend_pixel_go(&mut pixels[offset..offset + 4], color, coverage);
            }
        }
    }
}

fn transformed_path(path: &Path, transform: Transform) -> Option<Path> {
    path.clone().transform(transform)
}

fn exact_dash_parameters(path: &usvg::Path) -> Option<(f64, f64, f64)> {
    exact_dash_parameters_from_id(path.id())
}

fn exact_dash_parameters_for_path(
    path: &usvg::Path,
    raster_sources: &RasterSourceMap<'_>,
) -> Option<(f64, f64, f64)> {
    raster_sources
        .exact_dash_parameters
        .get(path.id())
        .copied()
        .or_else(|| exact_dash_parameters(path))
}

fn exact_dash_parameters_from_id(id: &str) -> Option<(f64, f64, f64)> {
    let marker = id.strip_prefix("d2rs-exact-dash-")?;
    let mut parts = marker.split('-');
    let dash = u64::from_str_radix(parts.next()?, 16).ok()?;
    let gap = u64::from_str_radix(parts.next()?, 16).ok()?;
    let offset = u64::from_str_radix(parts.next()?, 16).ok()?;
    // Synthetic sequence lifelines append their actor index to keep SVG IDs
    // unique; SVG salting may append another suffix. Animated connection
    // markers predate those suffixes and have only the three exact values.
    let _unique_suffixes = parts.collect::<Vec<_>>();
    Some((
        f64::from_bits(dash),
        f64::from_bits(gap),
        f64::from_bits(offset),
    ))
}

fn draw_path(
    path: &usvg::Path,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    text_outline: bool,
    reference_origin: Point64,
    rounding_origin: Point64,
    raster_sources: &RasterSourceMap<'_>,
    exact_svg_source: Option<&SvgPath>,
) -> bool {
    let (reference_origin, rounding_origin) = if raster_sources.banded {
        (
            Point64::default(),
            Point64 {
                x: rounding_origin.x - raster_sources.device_origin.x,
                y: rounding_origin.y - raster_sources.device_origin.y,
            },
        )
    } else {
        (reference_origin, rounding_origin)
    };
    draw_path_with_transform(
        path,
        pixels,
        width,
        height,
        scale,
        opacity,
        text_outline,
        None,
        reference_origin,
        rounding_origin,
        path_uses_analytic_paint(path),
        raster_sources,
        raster_sources.device_origin,
        exact_svg_source,
    )
}

fn path_scene_transform(
    path_transform: Transform,
    parent_transform: Option<Transform>,
) -> Transform {
    parent_transform.map_or(path_transform, |parent| path_transform.post_concat(parent))
}

fn draw_path_with_transform(
    path: &usvg::Path,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    text_outline: bool,
    parent_transform: Option<Transform>,
    reference_origin: Point64,
    rounding_origin: Point64,
    go_blend: bool,
    raster_sources: &RasterSourceMap<'_>,
    device_origin: Point64,
    exact_svg_source: Option<&SvgPath>,
) -> bool {
    let gradient_stops = raster_sources
        .native_gradient_paints
        .get(path.id())
        .unwrap_or(&raster_sources.gradient_stops);
    let _stroke_only = GoStrokeOnlyScope::new(path.fill().is_none() && path.stroke().is_some());
    let scene_transform = path_scene_transform(path.abs_transform(), parent_transform);
    let unshifted_device_transform = scene_transform.post_scale(scale, scale);
    let device_transform = Transform::from_row(
        unshifted_device_transform.sx,
        unshifted_device_transform.ky,
        unshifted_device_transform.kx,
        unshifted_device_transform.sy,
        unshifted_device_transform.tx - device_origin.x as f32,
        unshifted_device_transform.ty - device_origin.y as f32,
    );
    let unshifted_outer_transform = if parent_transform.is_none() {
        raster_sources
            .native_path_transforms
            .get(&(path as *const usvg::Path))
            .copied()
            .unwrap_or_else(|| canonical_transform(unshifted_device_transform))
    } else {
        canonical_transform(unshifted_device_transform)
    };
    // Source shifts the retained f64 device matrix into a cropped filter
    // layer, while prepared paints stay keyed by the unshifted matrix.
    let mut outer_transform = unshifted_outer_transform;
    outer_transform.tx -= device_origin.x;
    outer_transform.ty -= device_origin.y;
    let local_transform = || {
        raster_sources
            .get(path.id())
            .and_then(|source| source.transform)
            .map(|[sx, kx, ky, sy, tx, ty]| Affine64 {
                sx,
                kx,
                ky,
                sy,
                tx,
                ty,
            })
    };
    let transform = exact_svg_source
        .and_then(|source| {
            parent_transform
                .and_then(|parent| intrinsic_svg_transform(source, parent, scale, device_origin))
        })
        .unwrap_or_else(|| {
            local_transform()
                .map(|local| affine_mul(outer_transform, local))
                .unwrap_or(outer_transform)
        });
    let prepared_pattern_transform = exact_svg_source
        .and_then(|source| {
            parent_transform.and_then(|parent| {
                intrinsic_svg_transform(source, parent, scale, Point64::default())
            })
        })
        .unwrap_or_else(|| {
            local_transform()
                .map(|local| affine_mul(unshifted_outer_transform, local))
                .unwrap_or(unshifted_outer_transform)
        });
    // raster.go:renderNode rejects primitives outside the destination before
    // scan conversion. Bound native geometry before expanding its strokes;
    // imported paths with independent precision transport keep their own path.
    // Avoid preparing and retaining off-band stroke outlines for the rest
    // of a large frame (notably grids with double borders).
    if !go_band_recording()
        && let Some((first, end)) = frame_band_rows(width, height)
        && (raster_sources.native_rects.contains_key(path.id())
            || (raster_sources
                .native_path_transforms
                .contains_key(&(path as *const usvg::Path))
                && raster_sources.get(path.id()).is_none()
                && exact_svg_source.is_none()))
    {
        let [x, y, w, h] = raster_sources.native_rects.get(path.id()).map_or_else(
            || {
                let bounds = path.data().bounds();
                [bounds.x(), bounds.y(), bounds.width(), bounds.height()].map(f64::from)
            },
            |rect| rect.box_values,
        );
        let (top, bottom) = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
            .into_iter()
            .map(|(x, y)| map_point(Point64 { x, y }, transform).y)
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(top, bottom), y| {
                (top.min(y), bottom.max(y))
            });
        let stroke = path.stroke().map_or(0.0, |stroke| {
            let mut factor = if stroke.linecap() == usvg::LineCap::Square {
                std::f64::consts::SQRT_2
            } else {
                1.0
            };
            if matches!(
                stroke.linejoin(),
                usvg::LineJoin::Miter | usvg::LineJoin::MiterClip
            ) {
                factor = factor.max(f64::from(stroke.miterlimit().get()));
            }
            f64::from(stroke.width().get())
                * 0.5
                * factor
                * (transform.ky.abs() + transform.sy.abs())
        });
        // Include antialiasing, reference-relative f32 rounding and the
        // rounded origin's final translation. A nonfinite bound never culls.
        let coordinate_magnitude = (x.abs() + w.abs()) * transform.ky.abs()
            + (y.abs() + h.abs()) * transform.sy.abs()
            + transform.ty.abs();
        let margin = stroke
            + 4.0
            + (coordinate_magnitude + rounding_origin.y.abs() + height as f64)
                * f64::from(f32::EPSILON)
                * 4.0;
        if top.is_finite()
            && bottom.is_finite()
            && margin.is_finite()
            && (bottom + margin <= first as f64 || top - margin >= end as f64)
        {
            return true;
        }
    }
    // Preserve local coordinates through the scene transform. Transforming
    // through tiny-skia first rounds translated device coordinates to f32.
    let tolerance = 0.375;
    let source_tolerance = flatten_tolerance(device_transform);
    if text_outline && path.stroke().is_none() {
        let Some(fill) = path.fill() else { return true };
        if !matches!(
            fill.paint(),
            Paint::Color(_) | Paint::LinearGradient(_) | Paint::RadialGradient(_)
        ) && !matches!(fill.paint(), Paint::Pattern(pattern) if pattern::is_prepared(pattern, transform, device_origin, Some(prepared_pattern_transform), &raster_sources.pattern_tiles))
        {
            return false;
        }
        let subpaths = exact_svg_source
            .and_then(|source| collect_raw_subpaths(source, source_tolerance, transform))
            .unwrap_or_else(|| collect_subpaths(path.data(), source_tolerance, true));
        let mut rasterizer = Rasterizer::new(width, height, tolerance);
        rasterizer.set_trace_origin(device_origin);
        let mapped = subpaths
            .into_iter()
            .map(|subpath| map_centerline(subpath, transform))
            .collect::<Vec<_>>();
        // Source drawRasterizedPaint resets non-solid paint's rasterizer to
        // subpathPixelBounds (one-pixel expansion, floored), whereas solid
        // paint keeps the layer origin. Coverage accumulates relative to it.
        if !matches!(fill.paint(), Paint::Color(_)) && !raster_sources.banded {
            set_paint_bounds_origin(&mut rasterizer, &mapped);
        }
        for (points, closed) in &mapped {
            rasterizer.add_fill_polyline(points, *closed);
        }
        let paint = paint_sampler(
            fill.paint(),
            opacity * fill.opacity().get(),
            device_transform,
            &raster_sources.pattern_tiles,
            Some(transform),
            Some(prepared_pattern_transform),
            device_origin,
            gradient_stops,
        );
        // Source drawPaintCoverage composites every non-solid paint sample
        // with compositeNRGBAOverRGBA, not DrawRGBA's wide-alpha kernel.
        if go_blend || !matches!(fill.paint(), Paint::Color(_)) {
            rasterizer.paint_with_go_blend(pixels, paint);
        } else {
            rasterizer.paint_with(pixels, paint);
        }
        return true;
    }
    let line_key = (
        path as *const usvg::Path as usize,
        [
            transform.sx,
            transform.kx,
            transform.ky,
            transform.sy,
            transform.tx,
            transform.ty,
            source_tolerance,
        ]
        .map(f64::to_bits),
        [
            device_transform.sx,
            device_transform.kx,
            device_transform.ky,
            device_transform.sy,
            device_transform.tx,
            device_transform.ty,
        ]
        .map(f32::to_bits),
    );
    let prepare_lines = || {
        let ellipse_centerline =
            collect_ellipse_centerline(path.data(), device_transform, source_tolerance);
        #[cfg(feature = "diagnostic-traces")]
        if let Some((trace_x, trace_y)) = raster_trace_pixel()
            && let Some((points, closed)) = ellipse_centerline.as_ref()
        {
            let mapped = map_centerline((points.clone(), *closed), transform);
            let min_x = mapped
                .0
                .iter()
                .map(|point| point.x)
                .fold(f64::INFINITY, f64::min);
            let min_y = mapped
                .0
                .iter()
                .map(|point| point.y)
                .fold(f64::INFINITY, f64::min);
            let max_x = mapped
                .0
                .iter()
                .map(|point| point.x)
                .fold(f64::NEG_INFINITY, f64::max);
            let max_y = mapped
                .0
                .iter()
                .map(|point| point.y)
                .fold(f64::NEG_INFINITY, f64::max);
            if trace_x as f64 >= min_x - 2.0
                && trace_x as f64 <= max_x + 2.0
                && trace_y as f64 >= min_y - 2.0
                && trace_y as f64 <= max_y + 2.0
            {
                eprintln!(
                    "d2-rs-ellipse-centerline bounds=({min_x:.9},{min_y:.9},{max_x:.9},{max_y:.9}) points={} transform=({:.9},{:.9},{:.9},{:.9},{:.9},{:.9})",
                    points.len(),
                    transform.sx,
                    transform.kx,
                    transform.ky,
                    transform.sy,
                    transform.tx,
                    transform.ty,
                );
                for (index, (source, device)) in points.iter().zip(&mapped.0).enumerate() {
                    eprintln!(
                        "d2-rs-ellipse-point index={index} source=({:.17e},{:.17e}) device=({:.17e},{:.17e})",
                        source.x, source.y, device.x, device.y,
                    );
                }
            }
        }
        let source_subpaths = raster_sources
            .native_rects
            .get(path.id())
            .map(|rect| vec![collect_scene_rect_centerline(*rect, source_tolerance)])
            .or_else(|| {
                exact_svg_source
                    .and_then(|source| collect_raw_subpaths(source, source_tolerance, transform))
            })
            .or_else(|| {
                raster_sources
                    .get(path.id())
                    .and_then(|source| collect_raster_source_subpaths(source, source_tolerance))
            });
        let source_centerline = source_subpaths
            .as_ref()
            .filter(|subpaths| subpaths.len() == 1)
            .map(|subpaths| subpaths[0].clone());
        let centerline = source_centerline
            .or(ellipse_centerline)
            .or_else(|| collect_centerline(path.data(), source_tolerance));
        let centerlines =
            source_subpaths.or_else(|| centerline.as_ref().map(|line| vec![line.clone()]));
        std::rc::Rc::new(PreparedPathLines {
            centerline,
            centerlines,
        })
    };
    let lines = if TEXT_MEMO.with(|memo| {
        memo.borrow()
            .as_ref()
            .is_some_and(|memo| memo.raster_surface == Some((width, height)))
    }) {
        text_memo(|memo| &mut memo.path_lines, line_key, prepare_lines)
    } else {
        prepare_lines()
    };
    let centerline = &lines.centerline;
    let centerlines = &lines.centerlines;
    #[cfg(feature = "diagnostic-traces")]
    if raster_trace_pixel().is_some() && path.id().contains("native-raster-arrow") {
        eprintln!(
            "d2-rs-arrow-transform id={} scene=({:.17e},{:.17e},{:.17e},{:.17e},{:.17e},{:.17e}) device=({:.17e},{:.17e},{:.17e},{:.17e},{:.17e},{:.17e}) device_origin=({:.17e},{:.17e}) reference_origin=({:.17e},{:.17e}) rounding_origin=({:.17e},{:.17e})",
            path.id(),
            scene_transform.sx,
            scene_transform.kx,
            scene_transform.ky,
            scene_transform.sy,
            scene_transform.tx,
            scene_transform.ty,
            device_transform.sx,
            device_transform.kx,
            device_transform.ky,
            device_transform.sy,
            device_transform.tx,
            device_transform.ty,
            device_origin.x,
            device_origin.y,
            reference_origin.x,
            reference_origin.y,
            rounding_origin.x,
            rounding_origin.y,
        );
        if let Some(centerlines) = centerlines.as_ref() {
            for (subpath_index, (points, closed)) in centerlines.iter().enumerate() {
                let mapped = map_centerline((points.clone(), *closed), transform);
                for (point_index, (source, device)) in points.iter().zip(&mapped.0).enumerate() {
                    eprintln!(
                        "d2-rs-arrow-point id={} subpath={} point={} source=({:.17e},{:.17e}) device=({:.17e},{:.17e})",
                        path.id(),
                        subpath_index,
                        point_index,
                        source.x,
                        source.y,
                        device.x,
                        device.y,
                    );
                }
            }
        }
    }
    if path.fill().is_none() {
        if let Some(stroke) = path.stroke()
            && let Some((points, closed)) = centerline.as_ref()
        {
            let Some(color) = solid_paint_exact(
                stroke.paint(),
                opacity * stroke.opacity().get(),
                exact_svg_source
                    .and_then(|source| source.paint_opacities)
                    .map(|o| o[1]),
            ) else {
                return false;
            };
            #[cfg(feature = "diagnostic-traces")]
            if let Some((trace_x, trace_y)) = raster_trace_pixel()
                && (reference_origin.x != 0.0 || reference_origin.y != 0.0)
            {
                let mapped = map_centerline((points.clone(), *closed), transform);
                let min_x = mapped
                    .0
                    .iter()
                    .map(|point| point.x)
                    .fold(f64::INFINITY, f64::min);
                let min_y = mapped
                    .0
                    .iter()
                    .map(|point| point.y)
                    .fold(f64::INFINITY, f64::min);
                let max_x = mapped
                    .0
                    .iter()
                    .map(|point| point.x)
                    .fold(f64::NEG_INFINITY, f64::max);
                let max_y = mapped
                    .0
                    .iter()
                    .map(|point| point.y)
                    .fold(f64::NEG_INFINITY, f64::max);
                let margin = f64::from(stroke.width().get()) * 2.0 + 2.0;
                if (trace_x as f64) >= min_x - margin
                    && (trace_x as f64) <= max_x + margin
                    && (trace_y as f64) >= min_y - margin
                    && (trace_y as f64) <= max_y + margin
                {
                    eprintln!(
                        "d2-rs-centerline ref=({:.0},{:.0}) width={:.9} closed={} points={}",
                        reference_origin.x,
                        reference_origin.y,
                        stroke.width().get(),
                        closed,
                        points.len(),
                    );
                    for (index, (source, device)) in points.iter().zip(&mapped.0).enumerate() {
                        eprintln!(
                            "d2-rs-centerline-point index={index} source=({:.17e},{:.17e}) device-global=({:.17e},{:.17e}) device-local=({:.17e},{:.17e})",
                            source.x,
                            source.y,
                            device.x,
                            device.y,
                            device.x - reference_origin.x,
                            device.y - reference_origin.y,
                        );
                    }
                }
            }
            let mut rasterizer = Rasterizer::new(width, height, tolerance);
            rasterizer.set_trace_origin(device_origin);
            rasterizer.set_reference_origin(reference_origin);
            rasterizer.set_rounding_origin(rounding_origin);
            rasterizer.prepare_path(path, 1, transform, source_tolerance, |rasterizer| {
                if let Some(dashes) = stroke.dasharray() {
                    let cleaned_points = clean_stroke_points(points, *closed);
                    for run in Rasterizer::dashed_centerline_exact(
                        &cleaned_points,
                        *closed,
                        dashes,
                        stroke.dashoffset(),
                        exact_dash_parameters_for_path(path, raster_sources),
                    ) {
                        rasterizer.add_stroke_polyline(
                            &run.points,
                            transform,
                            f64::from(stroke.width().get()),
                            run.closed,
                            stroke.linejoin(),
                            f64::from(stroke.miterlimit().get()),
                            stroke.linecap(),
                        );
                    }
                } else {
                    rasterizer.add_stroke_polyline(
                        &clean_stroke_points(points, *closed),
                        transform,
                        f64::from(stroke.width().get()),
                        *closed,
                        stroke.linejoin(),
                        f64::from(stroke.miterlimit().get()),
                        stroke.linecap(),
                    );
                }
            });
            rasterizer.paint(pixels, color);
            return true;
        }
    }
    if let Some(fill) = path.fill() {
        let exact_color = solid_paint_exact(
            fill.paint(),
            opacity * fill.opacity().get(),
            exact_svg_source
                .and_then(|source| source.paint_opacities)
                .map(|o| o[0]),
        );
        if !matches!(
            fill.paint(),
            Paint::Color(_) | Paint::LinearGradient(_) | Paint::RadialGradient(_)
        ) && !matches!(fill.paint(), Paint::Pattern(pattern) if pattern::is_prepared(pattern, transform, device_origin, Some(prepared_pattern_transform), &raster_sources.pattern_tiles))
        {
            return false;
        }
        if fill.rule() == FillRule::EvenOdd {
            let subpaths = exact_svg_source
                .and_then(|source| collect_raw_subpaths(source, source_tolerance, transform))
                .unwrap_or_else(|| collect_subpaths(path.data(), source_tolerance, false));
            let mapped: Vec<_> = subpaths
                .into_iter()
                .map(|path| map_centerline(path, transform))
                .collect();
            let sample = paint_sampler(
                fill.paint(),
                opacity * fill.opacity().get(),
                device_transform,
                &raster_sources.pattern_tiles,
                Some(transform),
                Some(prepared_pattern_transform),
                device_origin,
                gradient_stops,
            );
            paint_even_odd_paths(pixels, width, height, &mapped, |x, y| {
                exact_color.or_else(|| sample(x, y))
            });
        } else {
            let mut rasterizer = Rasterizer::new(width, height, tolerance);
            rasterizer.set_trace_origin(device_origin);
            if raster_sources.contains_key(path.id())
                || raster_sources
                    .native_path_transforms
                    .contains_key(&(path as *const usvg::Path))
            {
                // Go rounds these native-scene fill vertices relative to the
                // owning masked group's crop origin. Keep that same origin for
                // the high-precision inline arrow paths instead of rounding the
                // transformed canvas coordinates directly.
                rasterizer.set_reference_origin(reference_origin);
                rasterizer.set_rounding_origin(rounding_origin);
            }
            rasterizer.prepare_path(path, 0, transform, source_tolerance, |rasterizer| {
                let mapped = if let Some(centerlines) = centerlines.as_ref() {
                    // The Go renderer flattens scene paths before applying the device
                    // transform. Reuse that centerline for simple fills as well as
                    // strokes so marker polygons and transformed curves do not pass
                    // through tiny-skia's f32 path transform first.
                    centerlines
                        .iter()
                        .map(|(points, closed)| {
                            map_centerline((points.clone(), *closed), transform)
                        })
                        .collect::<Vec<_>>()
                } else {
                    // Multi-subpath fills do not fit in `centerline`. Flatten each
                    // subpath with D2's scene-space tolerance before transforming it;
                    // the scanline kernel's looser device-space tolerance is intended
                    // only for paths submitted directly to its curve API.
                    collect_subpaths(path.data(), source_tolerance, false)
                        .into_iter()
                        .map(|subpath| map_centerline(subpath, transform))
                        .collect()
                };
                if exact_color.is_none() && !raster_sources.banded {
                    set_paint_bounds_origin(rasterizer, &mapped);
                }
                for (points, closed) in &mapped {
                    rasterizer.add_fill_polyline(points, *closed);
                }
            });
            let sample = paint_sampler(
                fill.paint(),
                opacity * fill.opacity().get(),
                device_transform,
                &raster_sources.pattern_tiles,
                Some(transform),
                Some(prepared_pattern_transform),
                device_origin,
                gradient_stops,
            );
            let paint = |x, y| exact_color.or_else(|| sample(x, y));
            if let Some(color) = exact_color.filter(|_| !go_blend) {
                rasterizer.paint(pixels, color);
            } else if go_blend || !matches!(fill.paint(), Paint::Color(_)) {
                rasterizer.paint_with_go_blend(pixels, paint);
            } else {
                rasterizer.paint_with(pixels, paint);
            }
        }
    }
    if let Some(stroke) = path.stroke()
        && let Some(centerlines) = centerlines.as_ref()
    {
        let gradient = matches!(
            stroke.paint(),
            Paint::LinearGradient(_) | Paint::RadialGradient(_)
        );
        let color = solid_paint_exact(
            stroke.paint(),
            opacity * stroke.opacity().get(),
            exact_svg_source
                .and_then(|source| source.paint_opacities)
                .map(|o| o[1]),
        );
        if color.is_none() && !gradient {
            return false;
        }
        let mut rasterizer = Rasterizer::new(width, height, tolerance);
        rasterizer.set_trace_origin(device_origin);
        rasterizer.set_reference_origin(reference_origin);
        rasterizer.set_rounding_origin(rounding_origin);
        rasterizer.prepare_path(path, 2, transform, source_tolerance, |rasterizer| {
            for (points, closed) in centerlines {
                if let Some(dashes) = stroke.dasharray() {
                    let cleaned_points = clean_stroke_points(points, *closed);
                    for run in Rasterizer::dashed_centerline_exact(
                        &cleaned_points,
                        *closed,
                        dashes,
                        stroke.dashoffset(),
                        exact_dash_parameters_for_path(path, raster_sources),
                    ) {
                        rasterizer.add_stroke_polyline(
                            &run.points,
                            transform,
                            f64::from(stroke.width().get()),
                            run.closed,
                            stroke.linejoin(),
                            f64::from(stroke.miterlimit().get()),
                            stroke.linecap(),
                        );
                    }
                } else {
                    rasterizer.add_stroke_polyline(
                        &clean_stroke_points(points, *closed),
                        transform,
                        f64::from(stroke.width().get()),
                        *closed,
                        stroke.linejoin(),
                        f64::from(stroke.miterlimit().get()),
                        stroke.linecap(),
                    );
                }
            }
        });
        if let Some(color) = color {
            rasterizer.paint(pixels, color);
        } else {
            // Source drawStroke samples non-solid stroke paint per pixel and
            // composites it with compositeNRGBAOverRGBA.
            rasterizer.paint_with_go_blend(
                pixels,
                paint_sampler(
                    stroke.paint(),
                    opacity * stroke.opacity().get(),
                    device_transform,
                    &raster_sources.pattern_tiles,
                    Some(transform),
                    Some(prepared_pattern_transform),
                    device_origin,
                    gradient_stops,
                ),
            );
        }
    } else if let Some(stroke) = path.stroke() {
        let Some(outline) = path.data().stroke(&stroke.to_tiny_skia(), 1.0) else {
            return false;
        };
        let Some(outline) = transformed_path(&outline, device_transform) else {
            return false;
        };
        let Some(color) = solid_paint_exact(
            stroke.paint(),
            opacity * stroke.opacity().get(),
            exact_svg_source
                .and_then(|source| source.paint_opacities)
                .map(|o| o[1]),
        ) else {
            return false;
        };
        let mut rasterizer = Rasterizer::new(width, height, tolerance);
        rasterizer.set_trace_origin(device_origin);
        rasterizer.set_reference_origin(reference_origin);
        rasterizer.set_rounding_origin(rounding_origin);
        rasterizer.add_path(&outline);
        rasterizer.paint(pixels, color);
    }
    true
}

/// Source drawRasterizedPaint resets a non-solid paint's rasterizer to
/// subpathPixelBounds(paths, transform, 0, canvas): the transformed points'
/// minimum less one pixel, floored and clamped to the canvas. Solid paint
/// keeps the layer origin, and coverage accumulates relative to that origin.
fn set_paint_bounds_origin(rasterizer: &mut Rasterizer, mapped: &[(Vec<Point64>, bool)]) {
    let (min_x, min_y) = mapped
        .iter()
        .flat_map(|(points, _)| points)
        .fold((f64::INFINITY, f64::INFINITY), |(x, y), point| {
            (x.min(point.x), y.min(point.y))
        });
    if min_x.is_finite() && min_y.is_finite() {
        rasterizer.set_reference_origin(Point64 {
            x: (min_x - 1.0).max(0.0).floor(),
            y: (min_y - 1.0).max(0.0).floor(),
        });
    }
}

fn add_text_decoration_to_rasterizer(
    path: &usvg::Path,
    rasterizer: &mut Rasterizer,
    scale: f32,
    device_origin: Point64,
) -> bool {
    let device_transform = path.abs_transform().post_scale(scale, scale);
    let mut transform = canonical_transform(device_transform);
    transform.tx -= device_origin.x;
    transform.ty -= device_origin.y;
    let source_tolerance = flatten_tolerance(device_transform);
    if let Some((points, closed)) = collect_centerline(path.data(), source_tolerance) {
        let mapped = map_centerline((points, closed), transform);
        rasterizer.add_fill_polyline(&mapped.0, mapped.1);
        return true;
    }
    let Some(data) = transformed_path(path.data(), device_transform) else {
        return false;
    };
    rasterizer.add_path(&data);
    true
}

fn add_text_decoration_with_transform(
    path: &usvg::Path,
    rasterizer: &mut Rasterizer,
    scale: f32,
    parent_transform: Transform,
    horizontal_bounds: Option<(f64, f64)>,
    device_origin: Point64,
) -> bool {
    let scene_transform = parent_transform.post_concat(path.abs_transform());
    let device_transform = scene_transform.post_scale(scale, scale);
    let mut transform = canonical_transform(device_transform);
    transform.tx -= device_origin.x;
    transform.ty -= device_origin.y;
    let source_tolerance = flatten_tolerance(device_transform);
    if let Some((left, right)) = horizontal_bounds {
        let bounds = path.data().bounds();
        let points = vec![
            Point64 {
                x: left,
                y: f64::from(bounds.top()),
            },
            Point64 {
                x: right,
                y: f64::from(bounds.top()),
            },
            Point64 {
                x: right,
                y: f64::from(bounds.bottom()),
            },
            Point64 {
                x: left,
                y: f64::from(bounds.bottom()),
            },
        ];
        let mapped = map_centerline((points, true), transform);
        rasterizer.add_fill_polyline(&mapped.0, mapped.1);
    } else {
        let Some((points, closed)) = collect_centerline(path.data(), source_tolerance) else {
            return false;
        };
        let mapped = map_centerline((points, closed), transform);
        rasterizer.add_fill_polyline(&mapped.0, mapped.1);
    }
    true
}

#[derive(Clone, Copy)]
enum FixedPathSegment {
    MoveTo(Point64),
    LineTo(Point64),
    QuadTo(Point64, Point64),
    CubicTo(Point64, Point64, Point64),
    Close,
}

struct FixedGlyphPath {
    segments: Vec<FixedPathSegment>,
}

#[derive(Clone, Copy)]
struct RawPoint {
    x: i32,
    y: i32,
}

#[derive(Clone, Copy)]
enum RawGlyphSegment {
    MoveTo(RawPoint),
    LineTo(RawPoint),
    QuadTo(RawPoint, RawPoint),
    CubicTo(RawPoint, RawPoint, RawPoint),
    Close,
}

struct RawGlyphCapture {
    segments: Vec<RawGlyphSegment>,
}

impl RawGlyphCapture {
    fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    fn point(x: f32, y: f32) -> RawPoint {
        RawPoint {
            x: f64::from(x).trunc() as i32,
            y: f64::from(y).trunc() as i32,
        }
    }
}

impl OutlineBuilder for RawGlyphCapture {
    fn move_to(&mut self, x: f32, y: f32) {
        self.segments
            .push(RawGlyphSegment::MoveTo(Self::point(x, y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.segments
            .push(RawGlyphSegment::LineTo(Self::point(x, y)));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.segments.push(RawGlyphSegment::QuadTo(
            Self::point(x1, y1),
            Self::point(x, y),
        ));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.segments.push(RawGlyphSegment::CubicTo(
            Self::point(x1, y1),
            Self::point(x2, y2),
            Self::point(x, y),
        ));
    }

    fn close(&mut self) {
        self.segments.push(RawGlyphSegment::Close);
    }
}

fn be_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes([
        *data.get(offset)?,
        *data.get(offset + 1)?,
    ]))
}

fn be_i16(data: &[u8], offset: usize) -> Option<i16> {
    Some(i16::from_be_bytes([
        *data.get(offset)?,
        *data.get(offset + 1)?,
    ]))
}

fn be_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *data.get(offset)?,
        *data.get(offset + 1)?,
        *data.get(offset + 2)?,
        *data.get(offset + 3)?,
    ]))
}

fn raw_glyf_record<'a>(face: &ttf_parser::Face<'a>, glyph_id: u16) -> Option<&'a [u8]> {
    let head = face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"head"))?;
    let loca = face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"loca"))?;
    let glyf = face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"glyf"))?;
    let format = be_i16(head, 50)?;
    let (start, end) = if format == 0 {
        let start = usize::from(be_u16(loca, usize::from(glyph_id) * 2)?) * 2;
        let end = usize::from(be_u16(loca, (usize::from(glyph_id) + 1) * 2)?) * 2;
        (start, end)
    } else if format == 1 {
        let start = usize::try_from(be_u32(loca, usize::from(glyph_id) * 4)?).ok()?;
        let end = usize::try_from(be_u32(loca, (usize::from(glyph_id) + 1) * 4)?).ok()?;
        (start, end)
    } else {
        return None;
    };
    (start <= end).then(|| glyf.get(start..end)).flatten()
}

fn raw_glyph_is_composite(face: &ttf_parser::Face<'_>, glyph_id: u16) -> Option<bool> {
    let record = raw_glyf_record(face, glyph_id)?;
    Some(be_i16(record, 0)? < 0)
}

fn raw_simple_glyph(face: &ttf_parser::Face<'_>, glyph_id: u16) -> Option<Vec<RawGlyphSegment>> {
    let mut capture = RawGlyphCapture::new();
    face.outline_glyph(ttf_parser::GlyphId(u16::from(glyph_id)), &mut capture)?;
    Some(capture.segments)
}

fn transform_raw_point(
    point: RawPoint,
    txx: i16,
    txy: i16,
    tyx: i16,
    tyy: i16,
    dx: i32,
    dy: i32,
) -> RawPoint {
    const HALF: i64 = 1 << 13;
    let x = i64::from(point.x);
    let y = i64::from(point.y);
    RawPoint {
        x: dx
            + ((x * i64::from(txx) + HALF) >> 14) as i32
            + ((y * i64::from(tyx) + HALF) >> 14) as i32,
        y: dy
            + ((x * i64::from(txy) + HALF) >> 14) as i32
            + ((y * i64::from(tyy) + HALF) >> 14) as i32,
    }
}

fn transform_raw_segment(
    segment: RawGlyphSegment,
    txx: i16,
    txy: i16,
    tyx: i16,
    tyy: i16,
    dx: i32,
    dy: i32,
) -> RawGlyphSegment {
    match segment {
        RawGlyphSegment::MoveTo(point) => {
            RawGlyphSegment::MoveTo(transform_raw_point(point, txx, txy, tyx, tyy, dx, dy))
        }
        RawGlyphSegment::LineTo(point) => {
            RawGlyphSegment::LineTo(transform_raw_point(point, txx, txy, tyx, tyy, dx, dy))
        }
        RawGlyphSegment::QuadTo(control, point) => RawGlyphSegment::QuadTo(
            transform_raw_point(control, txx, txy, tyx, tyy, dx, dy),
            transform_raw_point(point, txx, txy, tyx, tyy, dx, dy),
        ),
        RawGlyphSegment::CubicTo(control0, control1, point) => RawGlyphSegment::CubicTo(
            transform_raw_point(control0, txx, txy, tyx, tyy, dx, dy),
            transform_raw_point(control1, txx, txy, tyx, tyy, dx, dy),
            transform_raw_point(point, txx, txy, tyx, tyy, dx, dy),
        ),
        RawGlyphSegment::Close => RawGlyphSegment::Close,
    }
}

fn raw_go_glyph(
    face: &ttf_parser::Face<'_>,
    glyph_id: u16,
    depth: usize,
) -> Option<Vec<RawGlyphSegment>> {
    if depth > 32 {
        return None;
    }
    let record = raw_glyf_record(face, glyph_id)?;
    let contours = be_i16(record, 0)?;
    if contours >= 0 {
        return raw_simple_glyph(face, glyph_id);
    }
    if contours != -1 {
        return None;
    }
    let mut offset = 10usize;
    let mut segments = Vec::new();
    loop {
        let flags = be_u16(record, offset)?;
        let component = be_u16(record, offset + 2)?;
        offset += 4;
        let (dx, dy) = if flags & 2 != 0 {
            let dx = if flags & 1 != 0 {
                i32::from(be_i16(record, offset)?)
            } else {
                i32::from(i8::from_be_bytes([*record.get(offset)?]))
            };
            let dy = if flags & 1 != 0 {
                i32::from(be_i16(record, offset + 2)?)
            } else {
                i32::from(i8::from_be_bytes([*record.get(offset + 1)?]))
            };
            offset += if flags & 1 != 0 { 4 } else { 2 };
            (dx, dy)
        } else {
            return None;
        };
        let mut txx = 1i16 << 14;
        let mut txy = 0i16;
        let mut tyx = 0i16;
        let mut tyy = 1i16 << 14;
        if flags & 8 != 0 {
            txx = be_i16(record, offset)?;
            tyy = txx;
            offset += 2;
        } else if flags & 64 != 0 {
            txx = be_i16(record, offset)?;
            tyy = be_i16(record, offset + 2)?;
            offset += 4;
        } else if flags & 128 != 0 {
            txx = be_i16(record, offset)?;
            txy = be_i16(record, offset + 2)?;
            tyx = be_i16(record, offset + 4)?;
            tyy = be_i16(record, offset + 6)?;
            offset += 8;
        }
        let child = raw_go_glyph(face, component, depth + 1)?;
        segments.extend(
            child
                .into_iter()
                .map(|segment| transform_raw_segment(segment, txx, txy, tyx, tyy, dx, dy)),
        );
        if flags & 32 == 0 {
            break;
        }
    }
    Some(segments)
}

/// Builds a glyph outline at the same fixed-point boundary as D2's Go
/// rasterizer. `sfnt.LoadGlyph` returns 26.6 coordinates before the outline is
/// flattened. Keep the path in f64 after applying that quantization so the
/// glyph origin is not rounded back through tiny-skia's f32 representation.
struct FixedGlyphPathBuilder {
    segments: Vec<FixedPathSegment>,
    transform: Affine64,
    scale_x: f64,
    scale_y: f64,
}

impl FixedGlyphPathBuilder {
    fn new(transform: Affine64, scale_x: f64, scale_y: f64) -> Self {
        Self {
            segments: Vec::new(),
            transform,
            scale_x,
            scale_y,
        }
    }

    fn point(&self, design_x: f32, design_y: f32) -> Point64 {
        // sfnt constructs implicit TrueType midpoints with integer division,
        // truncating toward zero. Do the same before the affine transform.
        let original_x = f64::from(design_x);
        let original_y = f64::from(design_y);
        let design_x = original_x.trunc();
        let design_y = original_y.trunc();
        // Go's sfnt.LoadGlyph rounds the scaled outline coordinates to its
        // 26.6 grid before flattenGlyph adds the glyph origin. Keep that
        // quantization separate from the translation; rounding the combined
        // value changes fractional text origins by a pixel after the 2x
        // raster transform.
        let scaled_x = (design_x * self.scale_x * 64.0).round() / 64.0;
        let scaled_y = (design_y * self.scale_y * 64.0).round() / 64.0;
        Point64 {
            x: scaled_x + design_y * self.transform.kx + self.transform.tx,
            y: design_x * self.transform.ky + scaled_y + self.transform.ty,
        }
    }

    fn finish(self) -> FixedGlyphPath {
        FixedGlyphPath {
            segments: self.segments,
        }
    }
}

fn fixed_path_from_raw(
    raw_segments: &[RawGlyphSegment],
    transform: Affine64,
    scale_x: f64,
    scale_y: f64,
) -> FixedGlyphPath {
    let mut builder = FixedGlyphPathBuilder::new(transform, scale_x, scale_y);
    for segment in raw_segments {
        match *segment {
            RawGlyphSegment::MoveTo(point) => builder.move_to(point.x as f32, point.y as f32),
            RawGlyphSegment::LineTo(point) => builder.line_to(point.x as f32, point.y as f32),
            RawGlyphSegment::QuadTo(control, point) => builder.quad_to(
                control.x as f32,
                control.y as f32,
                point.x as f32,
                point.y as f32,
            ),
            RawGlyphSegment::CubicTo(control0, control1, point) => builder.curve_to(
                control0.x as f32,
                control0.y as f32,
                control1.x as f32,
                control1.y as f32,
                point.x as f32,
                point.y as f32,
            ),
            RawGlyphSegment::Close => builder.close(),
        }
    }
    builder.finish()
}

impl OutlineBuilder for FixedGlyphPathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.segments
            .push(FixedPathSegment::MoveTo(self.point(x, y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.segments
            .push(FixedPathSegment::LineTo(self.point(x, y)));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let control = self.point(x1, y1);
        let point = self.point(x, y);
        self.segments.push(FixedPathSegment::QuadTo(control, point));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let control1 = self.point(x1, y1);
        let control2 = self.point(x2, y2);
        let point = self.point(x, y);
        self.segments
            .push(FixedPathSegment::CubicTo(control1, control2, point));
    }

    fn close(&mut self) {
        self.segments.push(FixedPathSegment::Close);
    }
}

fn round_fixed(value: f64) -> f64 {
    (value * 64.0).round() / 64.0
}

fn round_svg_text_origin(value: f64) -> f64 {
    // d2-rs emits text chunk origins with six fractional decimal places in
    // SVG. usvg stores them in f32, which can turn e.g. 43.49875 into
    // 43.498748779296875 before this fixed-point reconstruction runs. Recover
    // the serialized scene value before adding Go's 26.6 glyph advances. Five
    // decimals are enough for the fixed advance products while avoiding the
    // one-micro-unit error introduced by f32 parsing.
    let fixed = (value * 64.0).round() / 64.0;
    if (value - fixed).abs() <= 0.00001 {
        fixed
    } else {
        (value * 100_000.0).round() / 100_000.0
    }
}

fn glyph_advance_unrounded(tree: &Tree, glyph: &usvg::layout::PositionedGlyph) -> Option<f64> {
    let glyph_id = u16::try_from(glyph.id.0).ok()?;
    tree.fontdb()
        .with_face_data(glyph.font, |data, face_index| {
            let face = ttf_parser::Face::parse(data, face_index).ok()?;
            let units = f64::from(face.units_per_em());
            let advance = f64::from(face.glyph_hor_advance(ttf_parser::GlyphId(glyph_id))?);
            Some(advance * f64::from(glyph.font_size()) / units)
        })?
}

#[derive(Clone, Copy)]
struct FixedGlyphPlacement {
    advance: f64,
    offset: f64,
}

/// Shape one usvg span with the same HarfBuzz-compatible fixed-point boundary
/// used by D2's v0.9.0 Go renderer. usvg's harfrust layout retains fractional
/// font-unit results as f32; D2 configures its shaper at ceil(ppem)*64 and
/// carries the resulting integer positions as fixed.Int26_6 values. The
/// rustybuzz port uses the same OpenType tables while letting us reproduce that
/// scale and rounding explicitly.
fn fixed_span_placements(
    tree: &Tree,
    span: &usvg::layout::Span,
) -> Option<std::rc::Rc<Vec<FixedGlyphPlacement>>> {
    let first = span.positioned_glyphs.first()?;
    let source = span
        .positioned_glyphs
        .iter()
        .map(|glyph| glyph.text.as_str())
        .collect::<String>();
    let key = FixedSpanKey {
        tree: tree as *const Tree as usize,
        font: first.font,
        size: first.font_size().to_bits(),
        source,
        glyphs: span
            .positioned_glyphs
            .iter()
            .map(|glyph| glyph.id.0)
            .collect(),
    };
    if let Some(cached) = TEXT_MEMO.with(|memo| {
        memo.borrow()
            .as_ref()
            .and_then(|memo| memo.fixed_spans.get(&key).cloned())
    }) {
        return cached;
    }
    let placements = fixed_span_placements_uncached(tree, span, &key.source).map(std::rc::Rc::new);
    text_memo_store(|memo| &mut memo.fixed_spans, key, placements.clone());
    placements
}

// Keep independent code-token shaping boundaries (code_shape.go), sharing
// only immutable results for the same face, scale, source and glyph order.
// The caller still anchors and advances every occurrence separately.
#[derive(Clone, Hash, PartialEq, Eq)]
struct FixedSpanKey {
    tree: usize,
    font: usvg::fontdb::ID,
    size: u32,
    source: String,
    glyphs: Vec<u32>,
}

fn fixed_span_placements_uncached(
    tree: &Tree,
    span: &usvg::layout::Span,
    source: &str,
) -> Option<Vec<FixedGlyphPlacement>> {
    let first = span.positioned_glyphs.first()?;
    tree.fontdb()
        .with_face_data(first.font, |data, face_index| {
            let face = rustybuzz::Face::from_slice(data, face_index)?;
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(source);
            buffer.guess_segment_properties();
            let shaped = rustybuzz::shape(&face, &[], buffer);
            if shaped.len() != span.positioned_glyphs.len()
                || shaped
                    .glyph_infos()
                    .iter()
                    .zip(span.positioned_glyphs.iter())
                    .any(|(info, glyph)| info.glyph_id != glyph.id.0)
            {
                return None;
            }
            let scale = f64::from(first.font_size().ceil()) * 64.0;
            let units = f64::from(face.units_per_em());
            let placements = shaped
                .glyph_positions()
                .iter()
                .enumerate()
                .map(|(index, position)| {
                    {
                        // go-text's HarfBuzz face scales the nominal
                        // hmtx advance and each GPOS adjustment
                        // independently. The nominal term is rounded by
                        // sfnt.scale, while emScaleX truncates the
                        // adjustment toward zero. Scaling the combined
                        // rustybuzz position loses a fixed-point unit at
                        // kerning boundaries (for example, `re`).
                        let nominal = face
                            .glyph_hor_advance(ttf_parser::GlyphId(
                                span.positioned_glyphs[index].id.0 as u16,
                            ))
                            .map(f64::from);
                        let advance = nominal.map_or_else(
                            || (f64::from(position.x_advance) * scale / units).round() / 64.0,
                            |nominal_design| {
                                let nominal = (nominal_design * scale / units).round() / 64.0;
                                let adjustment = f64::from(position.x_advance) - nominal_design;
                                nominal + (adjustment * scale / units).trunc() / 64.0
                            },
                        );
                        let offset = (f64::from(position.x_offset) * scale / units).trunc() / 64.0;
                        FixedGlyphPlacement { advance, offset }
                    }
                })
                .collect::<Vec<_>>();
            Some(placements)
        })?
}

fn fixed_nominal_advance(tree: &Tree, glyph: &usvg::layout::PositionedGlyph) -> Option<f64> {
    let glyph_id = u16::try_from(glyph.id.0).ok()?;
    tree.fontdb()
        .with_face_data(glyph.font, |data, face_index| {
            let face = ttf_parser::Face::parse(data, face_index).ok()?;
            let units = f64::from(face.units_per_em());
            let advance = f64::from(face.glyph_hor_advance(ttf_parser::GlyphId(glyph_id))?);
            let scale = f64::from(glyph.font_size().ceil()) * 64.0 / units;
            Some((advance * scale).round() / 64.0)
        })?
}

// d2raster/text.go uses the primary font's hhea descent, not a glyph's
// fallback face or OS/2 typographic metrics. Go sfnt rounds in Int26_6.
fn native_text_descent(tree: &Tree, text: &usvg::Text, font_size: f64) -> Option<f64> {
    let font = text.chunks().first()?.spans().first()?.font();
    let mut database = tree.fontdb().clone();
    let primary = usvg::FontResolver::default_font_selector()(font, &mut database)?;
    database.with_face_data(primary, |data, face_index| {
        let face = ttf_parser::Face::parse(data, face_index).ok()?;
        let hhea = face
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"hhea"))?;
        let descent = i64::from(i16::from_be_bytes(hhea.get(6..8)?.try_into().ok()?));
        let units = i64::from(face.units_per_em());
        let ppem = (font_size * 64.0).round() as i64;
        let product = descent.checked_mul(ppem)?;
        let rounded = if product >= 0 {
            product + units / 2
        } else {
            product - units / 2
        };
        Some(-(rounded / units) as f64 / 64.0)
    })?
}

fn affine_max_scale(transform: Affine64) -> f64 {
    let maximum = transform
        .sx
        .abs()
        .max(transform.kx.abs())
        .max(transform.ky.abs())
        .max(transform.sy.abs());
    if maximum == 0.0 || !maximum.is_finite() {
        return maximum;
    }
    let a = transform.sx / maximum;
    let b = transform.kx / maximum;
    let c = transform.ky / maximum;
    let d = transform.sy / maximum;
    let trace = a * a + b * b + c * c + d * d;
    let det = a * d - b * c;
    let disc = (trace * trace - 4.0 * det * det).max(0.0);
    maximum * ((trace + disc.sqrt()) / 2.0).max(0.0).sqrt()
}

fn add_native_text_underline(
    tree: &Tree,
    text: &usvg::Text,
    span: &usvg::layout::Span,
    glyph_transforms: &[Affine64],
    transform: Affine64,
    rasterizer: &mut Rasterizer,
    font_size: f64,
) -> Option<bool> {
    let (left, right) = fixed_decoration_bounds(tree, span, glyph_transforms)?;
    if right <= left {
        return Some(false);
    }
    let descent = native_text_descent(tree, text, font_size)?;
    let scale = affine_max_scale(transform);
    let min_thickness = if scale.is_finite() && scale > 1e-12 {
        1.0 / scale
    } else {
        1.0
    };
    let thickness = (font_size / 16.0).max(min_thickness);
    let top = glyph_transforms.first()?.ty + (font_size / 12.0).max(descent * 0.25);
    #[cfg(feature = "diagnostic-traces")]
    if raster_trace_pixel().is_some() {
        eprintln!(
            "d2-rs-native-underline size={font_size:.17e} descent={descent:.17e} thickness={thickness:.17e} top={top:.17e} left={left:.17e} right={right:.17e} transform=({:.17e},{:.17e},{:.17e},{:.17e},{:.17e},{:.17e})",
            transform.sx, transform.kx, transform.ky, transform.sy, transform.tx, transform.ty
        );
    }
    let points = vec![
        Point64 { x: left, y: top },
        Point64 { x: right, y: top },
        Point64 {
            x: right,
            y: top + thickness,
        },
        Point64 {
            x: left,
            y: top + thickness,
        },
    ];
    if points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return None;
    }
    let mapped = map_centerline((points, true), transform);
    rasterizer.add_fill_polyline(&mapped.0, mapped.1);
    Some(true)
}

fn fixed_decoration_bounds(
    tree: &Tree,
    span: &usvg::layout::Span,
    transforms: &[Affine64],
) -> Option<(f64, f64)> {
    let first = transforms.first()?;
    let last_index = transforms.len().checked_sub(1)?;
    let last_glyph = span.positioned_glyphs.get(last_index)?;
    let advance = fixed_nominal_advance(tree, last_glyph)?;
    Some((first.tx, transforms[last_index].tx + advance))
}

/// Go's text shaper carries glyph positions and advances in fixed.Int26_6.
/// HarfRust/usvg keeps the same values as f32, which leaves fractional
/// advances such as 8.448 instead of Go's 8.453125. Reconstruct the fixed
/// origin sequence from the usvg glyph order while preserving its kerning and
/// cluster transforms. This is intentionally limited to the outline path;
/// usvg's ordinary text layout and public SVG remain unchanged.
fn fixed_glyph_transforms(
    tree: &Tree,
    text: &usvg::Text,
    spans: &[usvg::layout::Span],
) -> Option<Vec<Vec<Affine64>>> {
    // Native code tokens share immutable shaping results below. Anchor each
    // occurrence on demand instead of retaining another absolute pen array
    // for every token in a long code block (code_shape.go buildCodeTextRuns).
    if text.id().starts_with("d2-rs-native-code-text-") {
        return fixed_glyph_transforms_uncached(tree, text, spans);
    }
    // The glyph layout already owns each linear transform. Retain only the
    // corrected pen positions, as d2raster/text.go's positioned glyphs do,
    // instead of another four f64 values for every glyph in a code block.
    let positions = text_memo(
        |memo| &mut memo.fixed,
        (
            text as *const usvg::Text as usize,
            spans.as_ptr() as usize,
            spans.len(),
        ),
        || {
            fixed_glyph_transforms_uncached(tree, text, spans).map(|transforms| {
                std::rc::Rc::new(
                    transforms
                        .into_iter()
                        .map(|span| {
                            span.into_iter()
                                .map(|t| Point64 { x: t.tx, y: t.ty })
                                .collect::<Vec<_>>()
                        })
                        .collect::<Vec<_>>(),
                )
            })
        },
    )?;
    Some(
        spans
            .iter()
            .zip(positions.iter())
            .map(|(span, positions)| {
                span.positioned_glyphs
                    .iter()
                    .zip(positions)
                    .map(|(glyph, position)| {
                        let t = glyph.outline_transform();
                        Affine64 {
                            sx: f64::from(t.sx),
                            kx: f64::from(t.kx),
                            ky: f64::from(t.ky),
                            sy: f64::from(t.sy),
                            tx: position.x,
                            ty: position.y,
                        }
                    })
                    .collect()
            })
            .collect(),
    )
}

fn fixed_glyph_transforms_uncached(
    tree: &Tree,
    text: &usvg::Text,
    spans: &[usvg::layout::Span],
) -> Option<Vec<Vec<Affine64>>> {
    if let [chunk] = text.chunks() {
        // With one chunk, every retained layouted span belongs to it even
        // if usvg omitted an unlayoutable source style span.
        return Some(fixed_glyph_transforms_for_chunk(tree, chunk, spans));
    }
    // usvg lays out chunks in source order and emits one layouted span for
    // each source style span. A chunk, rather than the whole <text> or each
    // style span, is the unit of text anchoring. D2's multiline labels use a
    // separate chunk for each independently centered line.
    //
    // Missing fonts or an empty/unlayoutable chunk can omit source spans.
    // Do not guess their association from glyph text or baseline coordinates;
    // let the ordinary text renderer handle an unsupported mapping instead.
    let source_span_count = text.chunks().iter().try_fold(0_usize, |count, chunk| {
        count.checked_add(chunk.spans().len())
    })?;
    if source_span_count != spans.len() {
        return None;
    }
    let mut output = Vec::with_capacity(spans.len());
    let mut span_offset = 0;
    for chunk in text.chunks() {
        let span_end = span_offset + chunk.spans().len();
        output.extend(fixed_glyph_transforms_for_chunk(
            tree,
            chunk,
            &spans[span_offset..span_end],
        ));
        span_offset = span_end;
    }
    Some(output)
}

fn fixed_glyph_transforms_for_chunk(
    tree: &Tree,
    chunk: &usvg::TextChunk,
    spans: &[usvg::layout::Span],
) -> Vec<Vec<Affine64>> {
    let glyphs = spans
        .iter()
        .flat_map(|span| span.positioned_glyphs.iter())
        .collect::<Vec<_>>();
    let Some(first) = glyphs.first() else {
        return vec![Vec::new(); spans.len()];
    };
    let first_transform = first.outline_transform();
    // The pinned Go renderer carries the parsed SVG text anchor into its
    // retained text run. Keep usvg's f32 chunk anchor as-is; reconstruct only
    // per-glyph advances below. Rounding this origin to 1/100000 moves the
    // shaped-grid cloud label enough to change edge coverage.
    let first_x = round_svg_text_origin(f64::from(first_transform.tx));
    let last = glyphs.last().expect("non-empty glyph list");
    let last_transform = last.outline_transform();
    let last_advance = glyph_advance_unrounded(tree, last).unwrap_or(0.0);
    let current_total =
        round_svg_text_origin(f64::from(last_transform.tx)) - first_x + last_advance;

    let anchor = chunk.anchor();
    let anchor_x = chunk.x().map(f64::from).unwrap_or_else(|| match anchor {
        usvg::TextAnchor::Start => first_x,
        usvg::TextAnchor::Middle => first_x + current_total * 0.5,
        usvg::TextAnchor::End => first_x + current_total,
    });

    let mut previous_prefix = 0.0;
    let mut previous_tx = first_x;
    let mut last_placement = None;
    let mut prefixes = Vec::with_capacity(glyphs.len());
    let mut global_index = 0;
    for span in spans {
        let placements = fixed_span_placements(tree, span);
        let first_span_tx = span
            .positioned_glyphs
            .first()
            .map(|glyph| f64::from(glyph.outline_transform().tx))
            .unwrap_or(previous_tx);
        let mut cursor = if global_index == 0 {
            0.0
        } else {
            // usvg preserves each styled span's scene origin. Re-anchor
            // at that origin instead of accumulating the preceding span's
            // f32 residue; Go's TextRun stores every span boundary at its
            // own fixed scene coordinate.
            round_svg_text_origin(first_span_tx) - first_x
        };
        for (span_index, glyph) in span.positioned_glyphs.iter().enumerate() {
            let transform = glyph.outline_transform();
            let placement = placements
                .as_ref()
                .and_then(|values| values.get(span_index))
                .copied();
            let position = cursor + placement.map_or(0.0, |value| value.offset);
            prefixes.push(position);
            previous_prefix = position;
            if let Some(value) = placement {
                cursor += value.advance;
                last_placement = Some(value);
            } else if span_index + 1 < span.positioned_glyphs.len() {
                let next = span.positioned_glyphs[span_index + 1].outline_transform();
                cursor += round_fixed(f64::from(next.tx) - f64::from(transform.tx));
            }
            previous_tx = round_svg_text_origin(f64::from(transform.tx));
            global_index += 1;
        }
    }
    let rounded_last_advance = last_placement
        .map(|placement| placement.advance)
        .unwrap_or_else(|| round_fixed(last_advance));
    let total = previous_prefix + rounded_last_advance;
    let first_corrected = match anchor {
        usvg::TextAnchor::Start => anchor_x,
        usvg::TextAnchor::Middle => anchor_x - total * 0.5,
        usvg::TextAnchor::End => anchor_x - total,
    };
    let mut cursor = 0;
    spans
        .iter()
        .map(|span| {
            span.positioned_glyphs
                .iter()
                .map(|glyph| {
                    let transform = glyph.outline_transform();
                    let transform = Affine64 {
                        sx: f64::from(transform.sx),
                        kx: f64::from(transform.kx),
                        ky: f64::from(transform.ky),
                        sy: f64::from(transform.sy),
                        tx: first_corrected + prefixes[cursor],
                        ty: round_svg_text_origin(f64::from(transform.ty)),
                    };
                    cursor += 1;
                    transform
                })
                .collect()
        })
        .collect()
}

fn apply_exact_text_origin(
    transforms: &mut [Vec<Affine64>],
    text: &usvg::Text,
    origin: SvgPoint,
    private_origin: bool,
) -> Option<(f64, f64)> {
    let first_transform = transforms.iter().flatten().next()?;
    let (x_delta, y_delta) = if private_origin {
        let [chunk] = text.chunks() else { return None };
        (
            origin.x - f64::from(chunk.x()?),
            // fixed_glyph_transforms rounds the layout baseline before it
            // applies relative glyph offsets. Subtract that same reference;
            // subtracting raw f32 y leaves its rounding residue on every line.
            origin.y - round_svg_text_origin(f64::from(chunk.y()?)),
        )
    } else {
        (0.0, origin.y - first_transform.ty)
    };
    for transform in transforms.iter_mut().flatten() {
        transform.tx += x_delta;
        transform.ty += y_delta;
    }
    Some((x_delta, y_delta))
}

fn apply_prepared_text_origins(
    transforms: &mut [Vec<Affine64>],
    text: &usvg::Text,
    origin: SvgPoint,
    private_origin: bool,
    sources: &SvgImagePathSources,
) -> Option<(f64, f64)> {
    let Some(tokens) = sources.code_line(text.id()) else {
        return apply_exact_text_origin(transforms, text, origin, private_origin);
    };
    if transforms.len() != tokens.len() {
        return None;
    }
    // Native tokens remain separate shaping chunks even when they share a
    // parser prototype. Preserve each source cursor and its f32 residue;
    // a single correction for the whole line loses it.
    for (placements, token) in transforms.iter_mut().zip(tokens) {
        let dx = token.origin.x - token.parsed_origin.x;
        let dy = token.origin.y - round_svg_text_origin(token.parsed_origin.y);
        for placement in placements {
            placement.tx += dx;
            placement.ty += dy;
        }
    }
    Some((0.0, 0.0))
}

fn fixed_glyph_path(
    tree: &Tree,
    glyph: &usvg::layout::PositionedGlyph,
    outline_transform: Affine64,
) -> Option<FixedGlyphPath> {
    let glyph_id = u16::try_from(glyph.id.0).ok()?;
    // d2raster/text.go retains its preparedFont outline reader and prepares
    // each glyph before drawing. Keep the design outline shared across text
    // runs and bands; quantization still happens at each placement's size.
    let outline = text_memo(
        |memo| &mut memo.glyph_outlines,
        (tree as *const Tree as usize, glyph.font, glyph_id),
        || {
            tree.fontdb()
                .with_face_data(glyph.font, |data, face_index| {
                    let face = ttf_parser::Face::parse(data, face_index).ok()?;
                    let units = f64::from(face.units_per_em());
                    let identity = Affine64 {
                        sx: 1.0,
                        sy: 1.0,
                        kx: 0.0,
                        ky: 0.0,
                        tx: 0.0,
                        ty: 0.0,
                    };
                    let path = if raw_glyph_is_composite(&face, glyph_id).unwrap_or(false) {
                        fixed_path_from_raw(&raw_go_glyph(&face, glyph_id, 0)?, identity, 1.0, 1.0)
                    } else {
                        let mut builder = FixedGlyphPathBuilder::new(identity, 1.0, 1.0);
                        face.outline_glyph(ttf_parser::GlyphId(glyph_id), &mut builder)?;
                        builder.finish()
                    };
                    Some(std::rc::Rc::new((units, path)))
                })
                .flatten()
        },
    )?;
    let (units, path) = outline.as_ref();
    let units = *units;
    let font_size = f64::from(glyph.font_size());
    let exact_size = round_fixed(font_size);
    let nominal_scale = font_size / units;
    let exact_scale = exact_size / units;
    let corrected_scale = |component: f64| {
        if (component.abs() - nominal_scale).abs() <= 1e-6 {
            component.signum() * exact_scale
        } else if nominal_scale > 0.0 {
            component / nominal_scale * exact_scale
        } else {
            component
        }
    };
    let mut builder = FixedGlyphPathBuilder::new(
        outline_transform,
        corrected_scale(outline_transform.sx),
        corrected_scale(outline_transform.sy),
    );
    for segment in &path.segments {
        match *segment {
            FixedPathSegment::MoveTo(p) => builder.move_to(p.x as f32, p.y as f32),
            FixedPathSegment::LineTo(p) => builder.line_to(p.x as f32, p.y as f32),
            FixedPathSegment::QuadTo(c, p) => {
                builder.quad_to(c.x as f32, c.y as f32, p.x as f32, p.y as f32)
            }
            FixedPathSegment::CubicTo(c0, c1, p) => builder.curve_to(
                c0.x as f32,
                c0.y as f32,
                c1.x as f32,
                c1.y as f32,
                p.x as f32,
                p.y as f32,
            ),
            FixedPathSegment::Close => builder.close(),
        }
    }
    Some(builder.finish())
}

thread_local! {
    static SCENE_FALLBACKS: std::cell::RefCell<
        Option<(usize, std::rc::Rc<HashMap<usvg::fontdb::ID, Vec<usvg::fontdb::ID>>>)>,
    > = const { std::cell::RefCell::new(None) };
}

fn reset_scene_fallback_buckets() {
    SCENE_FALLBACKS.with(|cache| *cache.borrow_mut() = None);
}

/// d2scenebuild resolveFontFallbacks (font_fallback.go:131–420): group every
/// text's uncovered grapheme clusters by its primary face, then resolve each
/// bucket once. Bundled faces keep precedence over host discovery.
fn scene_fallback_buckets(
    tree: &Tree,
) -> std::rc::Rc<HashMap<usvg::fontdb::ID, Vec<usvg::fontdb::ID>>> {
    use unicode_segmentation::UnicodeSegmentation;
    let key = tree as *const Tree as usize;
    if let Some(hit) = SCENE_FALLBACKS.with(|cache| {
        cache
            .borrow()
            .as_ref()
            .filter(|(cached, _)| *cached == key)
            .map(|(_, buckets)| buckets.clone())
    }) {
        return hit;
    }
    let mut database = tree.fontdb().clone();
    let selector = usvg::FontResolver::default_font_selector();
    let mut buckets: Vec<(usvg::fontdb::ID, Vec<char>)> = Vec::new();
    fn visit<'a>(group: &'a usvg::Group, texts: &mut Vec<&'a usvg::Text>) {
        for child in group.children() {
            match child {
                usvg::Node::Group(child) => visit(child, texts),
                usvg::Node::Text(text) => texts.push(text),
                _ => {}
            }
        }
    }
    let mut texts = Vec::new();
    visit(tree.root(), &mut texts);
    for text in &texts {
        for chunk in text.chunks() {
            let [style] = chunk.spans() else { continue };
            let Some(primary) = selector(style.font(), &mut database) else {
                continue;
            };
            for cluster in chunk.text().graphemes(true) {
                let covered = cluster.chars().all(|value| {
                    crate::font_shaping::ignorable(value)
                        || crate::font_fallback::face_covers(&database, primary, value)
                });
                if covered {
                    continue;
                }
                let index = match buckets.iter().position(|(id, _)| *id == primary) {
                    Some(index) => index,
                    None => {
                        buckets.push((primary, Vec::new()));
                        buckets.len() - 1
                    }
                };
                for value in cluster
                    .chars()
                    .filter(|value| !crate::font_shaping::ignorable(*value))
                {
                    if !buckets[index].1.contains(&value) {
                        buckets[index].1.push(value);
                    }
                }
            }
        }
    }
    let mut resolved = HashMap::new();
    for (primary, missing) in buckets {
        let mut ids = vec![primary];
        let mut remaining = Vec::new();
        for value in missing {
            match crate::bundled_emoji::loaded_face(value, &ids, &database) {
                Some(id) => {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                None => remaining.push(value),
            }
        }
        remaining.retain(|value| {
            !ids.iter()
                .any(|id| crate::font_fallback::face_covers(&database, *id, *value))
        });
        let system = crate::font_fallback::select_fallbacks(&remaining, &ids, &database);
        ids.extend(system);
        ids.remove(0);
        resolved.insert(primary, ids);
    }
    let resolved = std::rc::Rc::new(resolved);
    SCENE_FALLBACKS.with(|cache| *cache.borrow_mut() = Some((key, resolved.clone())));
    resolved
}

/// Rebuild mixed-font outline runs from the retained source chunks. usvg's
/// fallback can replace a complete line when a host face covers every scalar;
/// Source keeps covered graphemes on the primary face instead.
type NativeLayout = (Vec<usvg::layout::Span>, Vec<Vec<Affine64>>);
type NativeTextLayout = Option<std::rc::Rc<NativeLayout>>;

/// Text layout results of the current render, keyed by node address. A
/// banded frame draws every text once per band; shaping is deterministic
/// for a given tree, so each text is laid out once per render.
#[derive(Default)]
struct TextMemo {
    raster_surface: Option<(usize, usize)>,
    effect_layer: Option<ZeroedLayer>,
    // bands.go:setBandReferenceBounds fixes these before drawing bands.
    // Keep each immutable subtree's origin rather than revisiting its paths
    // and rebuilding dashed stroke runs for every destination pass.
    group_origins: HashMap<(usize, u32, bool), Point64>,
    rasters: HashMap<PreparedRasterKey, std::rc::Rc<PreparedRaster>>,
    path_lines: HashMap<PathLinesKey, std::rc::Rc<PreparedPathLines>>,
    glyph_outlines:
        HashMap<(usize, usvg::fontdb::ID, u16), Option<std::rc::Rc<(f64, FixedGlyphPath)>>>,
    native: HashMap<(usize, Option<String>), NativeTextLayout>,
    code_shapes: HashMap<
        (usize, usvg::fontdb::ID, u32, String),
        Option<std::rc::Rc<crate::font_shaping::Shaped>>,
    >,
    fixed_spans: HashMap<FixedSpanKey, Option<std::rc::Rc<Vec<FixedGlyphPlacement>>>>,
    fixed: HashMap<(usize, usize, usize), Option<std::rc::Rc<Vec<Vec<Point64>>>>>,
    color: HashMap<usize, bool>,
    color_glyph_presence: HashMap<(usize, usvg::fontdb::ID, u32), bool>,
    // d2raster/text.go preparedFont.bundledCOLRv1Plan and colrv1.go:
    // authenticate/compile each font glyph once, then prepare each placement
    // once instead of rebuilding its paint tree in every frame band.
    color_plans: HashMap<(usize, u32), Option<std::rc::Rc<crate::colr_plan::Plan>>>,
    color_glyphs: HashMap<ColorGlyphKey, Option<std::rc::Rc<colr_paint::PreparedGlyph>>>,
    color_paths: HashMap<(usize, u16, u64), Option<std::rc::Rc<Vec<Vec<Point64>>>>>,
    // Surface rows a text's rasterizers spanned when drawn with a given
    // placement (see draw_group_children).
    text_rows: HashMap<TextDrawKey, Option<(isize, isize)>>,
    // Appendix icon shadows still needed by later band passes.
    appendix: HashMap<AppendixShadowKey, Option<std::rc::Rc<AppendixShadow>>>,
}

// effects.go:offscreenBudget reuses the last exact-sized effect image.
// Reset its pixels before reuse, retaining the address mapping only.
fn take_effect_layer(len: usize) -> ZeroedLayer {
    let cached = TEXT_MEMO.with(|memo| {
        memo.borrow_mut()
            .as_mut()
            .and_then(|memo| memo.effect_layer.take())
    });
    if let Some(mut pixels) = cached.filter(|pixels| pixels.len() == len) {
        pixels.clear();
        return pixels;
    }
    ZeroedLayer::new(len)
}

fn recycle_effect_layer(
    pixels: ZeroedLayer,
    width: usize,
    height: usize,
    region: std::rc::Rc<LayerRegion>,
) {
    // Reuse ordinary rasterizer layers within one streamed pass; release
    // before advancing its band. Linux can reset anonymous mapping pages
    // without retaining the union of the layers' earlier frame positions.
    if cfg!(target_os = "linux") && region.tracked && frame_band_rows(width, height).is_some() {
        TEXT_MEMO.with(|memo| {
            if let Some(memo) = memo.borrow_mut().as_mut() {
                memo.effect_layer = Some(pixels);
            }
        });
    }
}

type TextDrawKey = (usize, [u64; 4], (usize, usize, u32, u32));
type ColorGlyphKey = (usize, u32, [u64; 6], usize, usize);

thread_local! {
    // While Some, paint_with_mode widens it to the surface rows its edges
    // span: (first row, end row).
    static PAINT_ROW_PROBE: std::cell::Cell<Option<(isize, isize)>> =
        const { std::cell::Cell::new(None) };
}

/// Store `value` in the current render's memo table, if there is one.
fn text_memo_store<K: std::hash::Hash + Eq, V>(
    table: fn(&mut TextMemo) -> &mut HashMap<K, V>,
    key: K,
    value: V,
) {
    TEXT_MEMO.with(|memo| {
        if let Some(memo) = memo.borrow_mut().as_mut() {
            table(memo).insert(key, value);
        }
    });
}

thread_local! {
    // Present only while a render owns it (TextMemoScope); trees are
    // immutable for that long, so node addresses are stable keys.
    static TEXT_MEMO: std::cell::RefCell<Option<TextMemo>> =
        const { std::cell::RefCell::new(None) };
}

struct TextMemoScope(Option<TextMemo>);

impl TextMemoScope {
    fn new() -> Self {
        Self(TEXT_MEMO.with(|memo| memo.replace(Some(TextMemo::default()))))
    }
}

impl Drop for TextMemoScope {
    fn drop(&mut self) {
        let previous = self.0.take();
        TEXT_MEMO.with(|memo| *memo.borrow_mut() = previous);
    }
}

/// Look `key` up in the current render's memo table, computing and keeping
/// it on a miss; outside a render, just compute it.
fn text_memo<K: std::hash::Hash + Eq, V: Clone>(
    table: fn(&mut TextMemo) -> &mut HashMap<K, V>,
    key: K,
    compute: impl FnOnce() -> V,
) -> V {
    let cached = TEXT_MEMO.with(|memo| {
        memo.borrow_mut()
            .as_mut()
            .map(|memo| table(memo).get(&key).cloned())
    });
    match cached {
        None => compute(),
        Some(Some(value)) => value,
        Some(None) => {
            let value = compute();
            TEXT_MEMO.with(|memo| {
                if let Some(memo) = memo.borrow_mut().as_mut() {
                    table(memo).insert(key, value.clone());
                }
            });
            value
        }
    }
}

fn native_mixed_font_layout(tree: &Tree, text: &usvg::Text) -> NativeTextLayout {
    native_mixed_font_layout_with_text(tree, text, None)
}

fn native_mixed_font_layout_with_text(
    tree: &Tree,
    text: &usvg::Text,
    source_text: Option<&str>,
) -> NativeTextLayout {
    // The precision transport often repeats the parser's text verbatim.
    // Share that layout with ordinary color/fallback discovery; only a real
    // one-chunk source override (e.g. retained control characters) changes
    // the shaping input and needs its own entry.
    let source_text =
        source_text.filter(|value| matches!(text.chunks(), [chunk] if chunk.text() != *value));
    text_memo(
        |memo| &mut memo.native,
        (
            text as *const usvg::Text as usize,
            source_text.map(str::to_owned),
        ),
        || native_mixed_font_layout_uncached(tree, text, source_text).map(std::rc::Rc::new),
    )
}

fn native_mixed_font_layout_with_sources(
    tree: &Tree,
    text: &usvg::Text,
    sources: &SvgImagePathSources,
) -> NativeTextLayout {
    let Some(code) = sources.code_line(text.id()) else {
        return native_mixed_font_layout_with_text(
            tree,
            text,
            sources.exact_text_content_by_id(text.id()),
        );
    };
    // d2scenebuild/code_shape.go keeps tokens independent. Retain shared
    // shaping results rather than another absolute glyph layout per line.
    // The parser supplies only face/paint prototypes for these native runs.
    if text.chunks().len() != text.layouted().len() {
        return None;
    }
    let mut spans = Vec::with_capacity(code.len());
    let mut placements = Vec::with_capacity(code.len());
    for token in code {
        let chunk = text.chunks().get(token.prototype)?;
        let prototype = text.layouted().get(token.prototype)?;
        let source = &token.text;
        let [style] = chunk.spans() else { return None };
        let [glyph] = prototype.positioned_glyphs.as_slice() else {
            return None;
        };
        if style.letter_spacing() != 0.0
            || style.word_spacing() != 0.0
            || style.text_length().is_some()
            || !matches!(chunk.text_flow(), usvg::TextFlow::Linear)
            || chunk.anchor() != usvg::TextAnchor::Start
        {
            return None;
        }
        let shaped = text_memo(
            |memo| &mut memo.code_shapes,
            (
                tree as *const Tree as usize,
                glyph.font,
                glyph.font_size().to_bits(),
                source.clone(),
            ),
            || {
                tree.fontdb()
                    .with_face_data(glyph.font, |data, index| {
                        let face = ttf_parser::Face::parse(data, index).ok()?;
                        if face.is_variable() {
                            return None;
                        }
                        crate::font_shaping::shape(
                            source,
                            f64::from(glyph.font_size()),
                            &[crate::font_shaping::Font { data, index }],
                        )
                        .map(std::rc::Rc::new)
                    })
                    .flatten()
            },
        )?;
        let prototype_transform = glyph.outline_transform();
        let x = token.parsed_origin.x;
        let y = round_svg_text_origin(token.parsed_origin.y);
        let mut span = prototype.clone();
        span.positioned_glyphs = Vec::with_capacity(shaped.glyphs.len());
        let mut positions = Vec::with_capacity(shaped.glyphs.len());
        for source in &shaped.glyphs {
            if source.empty || source.face != 0 || source.id == 0 {
                return None;
            }
            let mut glyph = glyph.clone();
            glyph.id = usvg::GlyphId(source.id);
            glyph.text.clone_from(&source.text);
            span.positioned_glyphs.push(glyph);
            positions.push(Affine64 {
                sx: f64::from(prototype_transform.sx),
                sy: f64::from(prototype_transform.sy),
                kx: f64::from(prototype_transform.kx),
                ky: f64::from(prototype_transform.ky),
                tx: x + source.x,
                ty: y + source.y,
            });
        }
        spans.push(span);
        placements.push(positions);
    }
    Some(std::rc::Rc::new((spans, placements)))
}

/// Font bytes handed to the shaper: the database's own shared buffer when it
/// has one, else a private copy.
enum ShapingFaceBytes {
    Shared(std::sync::Arc<dyn AsRef<[u8]> + Sync + Send>),
    Owned(Vec<u8>),
}

impl ShapingFaceBytes {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Shared(bytes) => (**bytes).as_ref(),
            Self::Owned(bytes) => bytes,
        }
    }
}

fn native_mixed_font_layout_uncached(
    tree: &Tree,
    text: &usvg::Text,
    source_text: Option<&str>,
) -> Option<NativeLayout> {
    use crate::font_shaping::{self, Font};
    if text.chunks().len() != text.layouted().len() {
        return None;
    }
    let mut database = tree.fontdb().clone();
    let selector = usvg::FontResolver::default_font_selector();
    let mut output = Vec::new();
    let mut transforms = Vec::new();
    let mut changed = false;
    for (chunk_index, (chunk, old)) in text.chunks().iter().zip(text.layouted()).enumerate() {
        let [style] = chunk.spans() else { return None };
        if style.letter_spacing() != 0.0
            || style.word_spacing() != 0.0
            || style.text_length().is_some()
            || !matches!(chunk.text_flow(), usvg::TextFlow::Linear)
        {
            return None;
        }
        let primary = selector(style.font(), &mut database)?;
        // usvg adds a wght setting even for static faces. Only a face with
        // real variation axes requires the unported variable-outline path.
        if !style.font().variations().is_empty()
            && database.with_face_data(primary, |data, index| {
                ttf_parser::Face::parse(data, index)
                    .ok()
                    .is_some_and(|face| face.is_variable())
            })?
        {
            return None;
        }
        let raw_text = source_text
            .filter(|_| text.chunks().len() == 1)
            .unwrap_or(chunk.text());
        let needs_native = raw_text != chunk.text()
            // Go's x/text segmenter retains embedding/override controls in
            // its runs; usvg's Unicode bidi layout can order them differently
            // even when every visible glyph uses the primary face.
            || raw_text.chars().any(|value| {
                font_shaping::ignorable(value)
                    || matches!(value, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
            || old
                .positioned_glyphs
                .iter()
                .any(|glyph| glyph.font != primary || glyph.id.0 == 0);
        if !needs_native {
            if changed {
                output.push(old.clone());
                transforms.push(
                    fixed_glyph_transforms_for_chunk(tree, chunk, std::slice::from_ref(old))
                        .remove(0),
                );
            }
            continue;
        }
        if !changed {
            // d2raster/text.go keeps one prepared glyph run. Ordinary code
            // tokens already have the primary face: leave their layout
            // borrowed, rather than cloning and positioning every preceding
            // chunk just to discard the result when no fallback is needed.
            for (chunk, old) in text.chunks().iter().zip(text.layouted()).take(chunk_index) {
                output.push(old.clone());
                transforms.push(
                    fixed_glyph_transforms_for_chunk(tree, chunk, std::slice::from_ref(old))
                        .remove(0),
                );
            }
            changed = true;
        }
        let mut ids = vec![primary];
        let mut missing = Vec::new();
        let mut seen_missing = std::collections::BTreeSet::new();
        if !raw_text.is_empty() {
            database.with_face_data(primary, |data, index| {
                let face = ttf_parser::Face::parse(data, index).ok();
                for value in raw_text.chars() {
                    let covered = face.as_ref().is_some_and(|face| {
                        crate::font_fallback::source_cmap::glyph_index(face, value)
                            .is_some_and(|id| face.glyph_index(value) == Some(id))
                    });
                    if !covered && seen_missing.insert(value) {
                        // Original resolveFontFallbacks walks graphemes in text
                        // order. The first discovered face can also cover a later
                        // rune, so sorting missing code points changes face
                        // ownership.
                        missing.push(value);
                    }
                }
            })?;
        }
        // d2scenebuild resolveFontFallbacks resolves one request per scene
        // font bucket; each text then takes the bucket's faces that cover one
        // of its own missing runes, in bucket order.
        let scene = (!missing.is_empty()).then(|| scene_fallback_buckets(tree));
        if let Some(fallbacks) = scene.as_ref().and_then(|scene| scene.get(&primary)) {
            for &id in fallbacks {
                if !ids.contains(&id)
                    && missing
                        .iter()
                        .any(|value| crate::font_fallback::face_covers(&database, id, *value))
                {
                    ids.push(id);
                }
            }
        }
        for value in missing {
            if ids
                .iter()
                .any(|id| crate::font_fallback::face_covers(&database, *id, value))
            {
                continue;
            }
            // Previously loaded bundled faces are available even though host
            // discovery excludes binary sources. Keep their source precedence.
            let bundled = old
                .positioned_glyphs
                .iter()
                .find(|glyph| glyph.text.contains(value))
                .map(|glyph| glyph.font)
                .filter(|id| !ids.contains(id))
                .filter(|id| {
                    database.face(*id).is_some_and(|face| {
                        matches!(face.source, usvg::fontdb::Source::Binary(_))
                            && crate::font_fallback::host_font_path(&face.source).is_none()
                    })
                });
            let fallback = crate::bundled_emoji::loaded_face(value, &ids, &database)
                .or(bundled)
                .or_else(|| crate::font_fallback::select_fallback(value, &ids, &database));
            if let Some(id) = fallback {
                ids.push(id);
            }
        }
        // Shape from the database's shared face bytes; only a face backed by
        // an unmapped file is read into a private copy.
        let data = ids
            .iter()
            .map(|&id| match database.face_source(id) {
                Some((usvg::fontdb::Source::Binary(bytes), index)) => {
                    Some((ShapingFaceBytes::Shared(bytes), index))
                }
                _ => database.with_face_data(id, |bytes, index| {
                    (ShapingFaceBytes::Owned(bytes.to_vec()), index)
                }),
            })
            .collect::<Option<Vec<_>>>()?;
        let fonts = data
            .iter()
            .map(|(bytes, index)| Font {
                data: bytes.as_slice(),
                index: *index,
            })
            .collect::<Vec<_>>();
        let units = data
            .iter()
            .map(|(bytes, index)| {
                ttf_parser::Face::parse(bytes.as_slice(), *index)
                    .ok()
                    .map(|face| face.units_per_em())
            })
            .collect::<Option<Vec<_>>>()?;
        let size = f64::from(style.font_size().get());
        let shaped = font_shaping::shape(raw_text, size, &fonts)?;
        let prototype = old.positioned_glyphs.first()?;
        let prototype_transform = prototype.outline_transform();
        let prototype_units = database.with_face_data(prototype.font, |bytes, index| {
            ttf_parser::Face::parse(bytes, index)
                .ok()
                .map(|face| face.units_per_em())
        })??;
        let anchor = chunk.x().map(f64::from)?;
        let first_x = anchor
            - match chunk.anchor() {
                usvg::TextAnchor::Start => 0.0,
                usvg::TextAnchor::Middle => shaped.advance * 0.5,
                usvg::TextAnchor::End => shaped.advance,
            };
        let baseline = round_svg_text_origin(
            chunk
                .y()
                .map_or(f64::from(prototype_transform.ty), f64::from),
        );
        let mut span = old.clone();
        span.positioned_glyphs.clear();
        let mut positions = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for source in shaped.glyphs {
            let mut glyph = prototype.clone();
            glyph.id = usvg::GlyphId(source.id);
            glyph.font = ids[source.face];
            glyph.text = if source.empty || !seen.insert(source.source_index) {
                String::new()
            } else {
                source.text
            };
            let ratio = f64::from(prototype_units) / f64::from(units[source.face]);
            positions.push(Affine64 {
                sx: f64::from(prototype_transform.sx) * ratio,
                sy: f64::from(prototype_transform.sy) * ratio,
                kx: f64::from(prototype_transform.kx) * ratio,
                ky: f64::from(prototype_transform.ky) * ratio,
                tx: first_x + source.x,
                ty: baseline + source.y,
            });
            span.positioned_glyphs.push(glyph);
        }
        output.push(span);
        transforms.push(positions);
    }
    changed.then_some((output, transforms))
}

/// Draws the simple, solid text subset with fixed-point glyph outlines. A
/// `None` result means that the text uses a feature this path does not model;
/// callers should then use usvg's flattened representation.
fn draw_text_fixed(
    text: &usvg::Text,
    tree: &Tree,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    reference_origin: Point64,
    text_reference_context: Option<PaintReferenceContext>,
    precise_device_transform: Option<Affine64>,
    device_origin: Point64,
    banded: bool,
    svg_text_origins: &SvgImagePathSources,
) -> Option<bool> {
    let native_layout = native_mixed_font_layout_with_sources(tree, text, svg_text_origins);
    if svg_text_origins.code_line(text.id()).is_some() && native_layout.is_none() {
        return None;
    }
    let spans = native_layout
        .as_deref()
        .map_or_else(|| text.layouted(), |(spans, _)| spans.as_slice());
    if spans.is_empty() {
        return None;
    }
    for span in spans {
        if !span.visible {
            return None;
        }
        let Some(fill) = span.fill.as_ref() else {
            continue;
        };
        // Gradient-filled runs take the same glyph outlines; decorations and
        // strokes on them remain on the established fallback.
        let gradient_run = matches!(
            fill.paint(),
            Paint::LinearGradient(_) | Paint::RadialGradient(_)
        ) && span.stroke.is_none()
            && span.underline.is_none()
            && span.overline.is_none()
            && span.line_through.is_none();
        if !matches!(fill.paint(), Paint::Color(_)) && !gradient_run {
            return None;
        }
        if let Some(stroke) = span.stroke.as_ref()
            && (stroke.dasharray().is_some() || !matches!(stroke.paint(), Paint::Color(_)))
        {
            return None;
        }
    }

    let parent_transform = text.abs_transform();
    #[cfg(feature = "diagnostic-traces")]
    if raster_trace_pixel().is_some() {
        eprintln!(
            "d2-rs-text-transform id={:?} abs={:?} precise={:?}",
            text.id(),
            parent_transform,
            precise_device_transform.map(|t| (t.sx, t.sy, t.tx, t.ty))
        );
    }
    let device_transform = parent_transform.post_scale(scale, scale);
    let synthetic_pivot = synthetic_italic_pivot(text);
    let mut transform = precise_device_transform.unwrap_or_else(|| {
        synthetic_pivot
            .and_then(|(x, y)| synthetic_italic_device_transform(device_transform, x, y))
            .unwrap_or_else(|| canonical_transform(device_transform))
    });
    // Text origins retain the serialized f32 translation; rounding them to
    // the generic scene snap loses the sub-pixel placement used by Go's
    // retained TextRun matrix.
    if synthetic_pivot.is_none() && precise_device_transform.is_none() {
        transform.tx = f64::from(device_transform.tx);
        transform.ty = f64::from(device_transform.ty);
    }
    transform.tx -= device_origin.x;
    transform.ty -= device_origin.y;
    let source_tolerance = flatten_tolerance(device_transform);
    let mut fixed_transforms = if let Some((_, transforms)) = native_layout.as_deref() {
        transforms.clone()
    } else {
        fixed_glyph_transforms(tree, text, spans)?
    };
    let text_source = spans
        .iter()
        .flat_map(|span| span.positioned_glyphs.iter())
        .map(|glyph| glyph.text.as_str())
        .collect::<String>();
    let first_glyph = spans
        .iter()
        .flat_map(|span| span.positioned_glyphs.iter())
        .next();
    let text_origin_query = first_glyph.map(|first_glyph| {
        (
            text.chunks()
                .first()
                .and_then(|chunk| chunk.x())
                .map_or_else(|| f64::from(first_glyph.outline_transform().tx), f64::from),
            f64::from(first_glyph.outline_transform().ty),
        )
    });
    let private_text_origin = svg_text_origins.exact_text_origin_by_id(text.id());
    let reference_origin = text_reference_origin(
        reference_origin,
        private_text_origin.is_some(),
        text_reference_context,
    );
    let rounding_origin = Point64 {
        x: reference_origin.x - device_origin.x,
        y: reference_origin.y - device_origin.y,
    };
    let mut reference_origin = if banded {
        Point64::default()
    } else {
        reference_origin
    };
    let exact_text_origin = private_text_origin.or_else(|| {
        text_origin_query.and_then(|(near_x, near_y)| {
            svg_text_origins.exact_text_origin(&text_source, near_x, near_y)
        })
    });
    #[cfg(feature = "diagnostic-traces")]
    if raster_trace_pixel().is_some() {
        eprintln!(
            "d2-rs-text-origin-query text={text_source:?} query={text_origin_query:?} abs_transform={:?} reference_origin=({:.17e},{:.17e}) found={exact_text_origin:?}",
            text.abs_transform(),
            reference_origin.x,
            reference_origin.y,
        );
    }
    if let Some(origin) = exact_text_origin
        && fixed_transforms.iter().flatten().next().is_some()
    {
        let (_, _y_delta) = apply_prepared_text_origins(
            &mut fixed_transforms,
            text,
            origin,
            private_text_origin.is_some(),
            svg_text_origins,
        )?;
        #[cfg(feature = "diagnostic-traces")]
        if raster_trace_pixel().is_some() {
            eprintln!(
                "d2-rs-text-source text={text_source:?} origin=({:.17e},{:.17e}) y_delta={_y_delta:.17e}",
                origin.x, origin.y,
            );
        }
    }
    // A source fallback can have ink outside usvg's original glyph bounds.
    // d2raster.prepareText/finishPrimitive bounds the resolved outlines, so
    // the inherited crop must also contain the replacement glyphs. Otherwise
    // reference_point puts their upper/left edges before the raster buffer.
    // Go prepares the fallback contours once for both bounds and paint.
    // Keep these only until this text is drawn, rather than flattening its
    // replacement glyphs again after finding their reference bounds.
    let mut prepared_fallbacks =
        (native_layout.is_some() && !banded && svg_text_origins.code_line(text.id()).is_none())
            .then(|| {
                spans
                    .iter()
                    .zip(&fixed_transforms)
                    .map(|(span, transforms)| {
                        span.positioned_glyphs
                            .iter()
                            .zip(transforms)
                            .map(|(glyph, &placement)| {
                                fixed_glyph_path(tree, glyph, placement)
                                    .map(|path| collect_fixed_subpaths(&path, source_tolerance))
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            });
    if let Some(prepared) = &prepared_fallbacks {
        for paths in prepared {
            for subpaths in paths.iter().flatten() {
                for (points, _) in subpaths {
                    for point in points {
                        let point = map_point(*point, transform);
                        reference_origin.x = reference_origin.x.min(point.x.floor() - 1.0);
                        reference_origin.y = reference_origin.y.min(point.y.floor() - 1.0);
                    }
                }
            }
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    if raster_trace_pixel().is_some() {
        for (span_index, span) in spans.iter().enumerate() {
            let span_text = span
                .positioned_glyphs
                .iter()
                .map(|glyph| glyph.text.as_str())
                .collect::<String>();
            eprintln!(
                "d2-rs-text span={span_index} text={span_text:?} glyphs={}",
                span.positioned_glyphs.len()
            );
            for (glyph_index, glyph) in span.positioned_glyphs.iter().enumerate() {
                let transform = glyph.outline_transform();
                eprintln!(
                    "d2-rs-text-glyph span={span_index} index={glyph_index} id={} font_size={:.17e} transform=({:.17e},{:.17e}) fixed_transform={:?}",
                    glyph.id.0,
                    glyph.font_size(),
                    transform.tx,
                    transform.ty,
                    fixed_transforms
                        .get(span_index)
                        .and_then(|transforms| transforms.get(glyph_index))
                        .map(|transform| (transform.tx, transform.ty)),
                );
            }
        }
    }
    let mut drew = false;
    for (span_index, span) in spans.iter().enumerate() {
        let Some(fill) = span.fill.as_ref() else {
            continue;
        };
        let solid = solid_paint(fill.paint(), opacity * fill.opacity().get());
        let color = solid.unwrap_or([0; 4]);
        let stroke = span.stroke.as_ref();
        let stroke_color =
            stroke.and_then(|stroke| solid_paint(stroke.paint(), opacity * stroke.opacity().get()));
        // Connection labels use the browser-link blue that D2's edge scene
        // painter reserves for linked edges. Native Markdown links use the
        // theme's darker blue and retain usvg's ordinary decoration path.
        let connection_underline = color[0] == 0 && color[1] == 0 && color[2] == 255;
        let native_underline = private_text_origin.is_some() && span.underline.is_some();
        let mut rasterizer = Rasterizer::new(width, height, 0.375);
        // Go's retained renderer rounds glyph coordinates relative to the
        // owning text primitive's reference bounds. Keep that boundary even
        // when this renderer paints into a full-canvas buffer.
        rasterizer.set_reference_origin(reference_origin);
        if banded {
            rasterizer.set_rounding_origin(rounding_origin);
        }
        let fixed_underline_bounds = fixed_transforms
            .get(span_index)
            .and_then(|transforms| {
                connection_underline.then(|| fixed_decoration_bounds(tree, span, transforms))
            })
            .flatten();
        for (decoration, transform, horizontal_bounds) in [
            (
                if native_underline {
                    None
                } else {
                    span.underline.as_ref()
                },
                if connection_underline {
                    underline_transform(span.underline.as_ref(), span, text.abs_transform())
                } else {
                    text.abs_transform().post_translate(0.0, 1.0)
                },
                fixed_underline_bounds,
            ),
            (span.overline.as_ref(), text.abs_transform(), None),
        ] {
            let Some(decoration) = decoration else {
                continue;
            };
            if connection_underline {
                if !add_text_decoration_with_transform(
                    decoration,
                    &mut rasterizer,
                    scale,
                    transform,
                    horizontal_bounds,
                    device_origin,
                ) {
                    return None;
                }
            } else if !draw_path_with_transform(
                decoration,
                pixels,
                width,
                height,
                scale,
                opacity,
                false,
                Some(transform),
                Point64 { x: 0.0, y: 0.0 },
                Point64 { x: 0.0, y: 0.0 },
                false,
                &RasterSourceMap::default(),
                device_origin,
                None,
            ) {
                return None;
            }
            drew = true;
        }
        let mut stroke_rasterizer = stroke.map(|_| Rasterizer::new(width, height, 0.375));
        if let Some(stroke_rasterizer) = stroke_rasterizer.as_mut() {
            stroke_rasterizer.set_reference_origin(reference_origin);
            if banded {
                stroke_rasterizer.set_rounding_origin(rounding_origin);
            }
        }
        let mut span_has_outline = false;
        let mut gradient_outlines = Vec::new();
        for (glyph_index, glyph) in span.positioned_glyphs.iter().enumerate() {
            let outline_transform = fixed_transforms
                .get(span_index)
                .and_then(|transforms| transforms.get(glyph_index))
                .copied()
                .unwrap_or_else(|| {
                    let transform = glyph.outline_transform();
                    Affine64 {
                        sx: f64::from(transform.sx),
                        kx: f64::from(transform.kx),
                        ky: f64::from(transform.ky),
                        sy: f64::from(transform.sy),
                        tx: round_svg_text_origin(f64::from(transform.tx)),
                        ty: round_svg_text_origin(f64::from(transform.ty)),
                    }
                });
            let subpaths = if let Some(prepared) = &mut prepared_fallbacks {
                prepared[span_index][glyph_index].take()
            } else {
                fixed_glyph_path(tree, glyph, outline_transform)
                    .map(|path| collect_fixed_subpaths(&path, source_tolerance))
            };
            let Some(subpaths) = subpaths else {
                // A whitespace glyph has advance but no outline. Keep it in
                // this fixed-point run so a single space does not force the
                // entire text node through usvg's f32 fallback.
                if glyph
                    .text
                    .chars()
                    .all(|value| value.is_whitespace() || crate::font_shaping::ignorable(value))
                {
                    continue;
                }
                // Bitmap/color glyphs and malformed outlines remain on
                // usvg's established fallback path.
                return None;
            };
            if subpaths.is_empty() {
                continue;
            }
            for (points, closed) in subpaths {
                if let (Some(stroke_rasterizer), Some(stroke)) =
                    (stroke_rasterizer.as_mut(), stroke)
                {
                    // d2raster.addStrokeRun constructs the outline in the
                    // glyph's local coordinates, then transforms its vertices.
                    // Stroking an already mapped centerline changes rounding
                    // and cannot retain nonuniform or synthetic-italic axes.
                    stroke_rasterizer.add_stroke_polyline(
                        &points,
                        transform,
                        svg_text_origins.text_stroke_width(text.id(), stroke.width().get()),
                        closed,
                        stroke.linejoin(),
                        f64::from(stroke.miterlimit().get()),
                        stroke.linecap(),
                    );
                }
                let mapped = map_centerline((points, closed), transform);
                if solid.is_some() {
                    rasterizer.add_fill_polyline(&mapped.0, mapped.1);
                } else {
                    gradient_outlines.push(mapped);
                }
            }
            span_has_outline = true;
        }
        if solid.is_none() && span_has_outline {
            // Source drawRasterizedPaint: non-solid text paint rasterizes
            // from its glyphs' own pixel bounds and composites each sample
            // with compositeNRGBAOverRGBA.
            if !banded {
                set_paint_bounds_origin(&mut rasterizer, &gradient_outlines);
            }
            for (points, closed) in &gradient_outlines {
                rasterizer.add_fill_polyline(points, *closed);
            }
            let tiles = PatternInstances::default();
            rasterizer.paint_with_go_blend(pixels, |x, y| {
                paint_color_at(
                    fill.paint(),
                    opacity * fill.opacity().get(),
                    device_transform,
                    x,
                    y,
                    &tiles,
                    Some(transform),
                    None,
                    device_origin,
                    svg_text_origins.gradient_stops(),
                )
            });
            drew = true;
            continue;
        }
        // Source appends underline after glyph contours and paints the entire
        // path set once, preserving overlap winding and opacity.
        if native_underline {
            span_has_outline |= add_native_text_underline(
                tree,
                text,
                span,
                fixed_transforms.get(span_index)?,
                transform,
                &mut rasterizer,
                svg_text_origins.text_font_size(text.id(), span.font_size.get()),
            )?;
        }
        // D2's TextRun preparation appends strike decoration to the same
        // filled path set as the glyphs. Painting it in the same rasterizer
        // preserves the source operation order and avoids a second source-over
        // pass where the decoration crosses a glyph.
        if let Some(decoration) = span.line_through.as_ref() {
            if !add_text_decoration_to_rasterizer(decoration, &mut rasterizer, scale, device_origin)
            {
                return None;
            }
            span_has_outline = true;
        }
        if span_has_outline {
            rasterizer.paint(pixels, color);
            if let (Some(stroke_rasterizer), Some(stroke_color)) =
                (stroke_rasterizer.as_mut(), stroke_color)
            {
                stroke_rasterizer.paint(pixels, stroke_color);
            }
            drew = true;
        }
    }
    Some(drew)
}

/// Match the v0.9.0 Go rasterizer's underline placement and one-scene-pixel
/// minimum thickness. usvg materializes text decorations as an 0.8px path;
/// D2's retained text run instead computes a 1px (for 16px text) bar from the
/// glyph baseline. The decoration path is already in scene coordinates, so a
/// direct y transform preserves its horizontal extent and antialiasing.
fn underline_transform(
    decoration: Option<&usvg::Path>,
    span: &usvg::layout::Span,
    text_transform: Transform,
) -> Transform {
    let Some(decoration) = decoration else {
        return text_transform;
    };
    if !decoration.abs_transform().is_identity() {
        return text_transform.post_translate(0.0, 1.0);
    }
    let Some(first) = span.positioned_glyphs.first() else {
        return text_transform.post_translate(0.0, 1.0);
    };
    let bounds = decoration.data().bounds();
    let current_height = bounds.height();
    if current_height <= f32::EPSILON {
        return text_transform.post_translate(0.0, 1.0);
    }
    let font_size = first.font_size();
    let thickness = (font_size / 16.0).max(1.0 / 2.0);
    let baseline = first.outline_transform().ty;
    let top = baseline + font_size / 12.0;
    let center = top + thickness / 2.0;
    let scale_y = thickness / current_height;
    let local = Transform::from_row(
        1.0,
        0.0,
        0.0,
        scale_y,
        0.0,
        center - ((bounds.top() + bounds.bottom()) / 2.0) * scale_y,
    );
    // `pre_concat` applies the local adjustment before the text's accumulated
    // scene transform, preserving the placement of nested/translated labels.
    text_transform.pre_concat(local)
}

fn draw_group(
    tree: &Tree,
    group: &usvg::Group,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    text_outline: bool,
    raster_sources: &RasterSourceMap<'_>,
    svg_image_paths: &SvgImagePathSources,
) -> bool {
    draw_group_impl(
        tree,
        group,
        pixels,
        width,
        height,
        scale,
        opacity,
        text_outline,
        false,
        None,
        false,
        None,
        raster_sources,
        svg_image_paths,
    )
}

fn draw_group_impl(
    tree: &Tree,
    group: &usvg::Group,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    text_outline: bool,
    ignore_masks: bool,
    inherited_reference_origin: Option<Point64>,
    inside_masked_layer: bool,
    effect_path_reference_origin: Option<Point64>,
    raster_sources: &RasterSourceMap<'_>,
    svg_image_paths: &SvgImagePathSources,
) -> bool {
    // Source renderNode gives an effect node (opacity, blend, isolation,
    // filters, clip or mask) its own layer; under RenderBands the layer is
    // tightened per band by effectRenderBounds. Record the subtree's leaf
    // geometry first, then draw it through those destinations.
    let effect = group.opacity().get() < 1.0
        || group.blend_mode() != usvg::BlendMode::Normal
        || group.isolate()
        || !group.filters().is_empty()
        || (group
            .clip_path()
            .is_some_and(|clip| !is_trivial_clip(clip, group))
            // The serialized canvas viewport is not a scene clip: source
            // paints the document's nodes directly onto the band canvas.
            && !group.id().starts_with("d2-rs-native-canvas-viewport"))
        || group.mask().is_some();
    let band_emulation = effect
        && go_band_frame_active(width, height)
        && (pixels.len() == width * height * 4 || layer_output(pixels).is_some());
    let draw = |pixels: &mut [u8]| {
        draw_group_impl_inner(
            tree,
            group,
            pixels,
            width,
            height,
            scale,
            opacity,
            text_outline,
            ignore_masks,
            inherited_reference_origin,
            inside_masked_layer,
            effect_path_reference_origin,
            raster_sources,
            svg_image_paths,
        )
    };
    if !band_emulation {
        return draw(pixels);
    }
    let key = group as *const usvg::Group;
    if go_band_recording() {
        // Nested inside an enclosing record pass: capture this subtree's
        // leaves for its own layer, and contribute them to the parent.
        GO_BAND_RECORD.with(|record| record.borrow_mut().push(Vec::new()));
        let recorded = draw(pixels);
        let mut leaves = GO_BAND_RECORD
            .with(|record| record.borrow_mut().pop())
            .unwrap_or_default();
        // effectRenderBounds keeps a filtered node's whole visible bounds,
        // which prepareFilters expanded by the filter region.
        if !group.filters().is_empty() {
            let bounds = group.abs_layer_bounding_box();
            let s = f64::from(scale);
            leaves.push((
                GoLeafBounds::Rect(
                    f64::from(bounds.left()) * s,
                    f64::from(bounds.top()) * s,
                    f64::from(bounds.right()) * s,
                    f64::from(bounds.bottom()) * s,
                ),
                None,
            ));
        }
        go_clip_leaves(
            &mut leaves,
            go_group_clip_rect(
                group,
                scale,
                svg_image_paths,
                raster_sources.native_canvas_size(width, height),
            ),
        );
        GO_BAND_RECORD.with(|record| {
            if let Some(parent) = record.borrow_mut().last_mut() {
                parent.extend(leaves.iter().cloned());
            }
        });
        GO_BAND_LEAF_CACHE.with(|cache| cache.borrow_mut().insert(key, std::rc::Rc::new(leaves)));
        return recorded;
    }
    let layer_key = GO_BAND_FRAME.with(|frame| {
        let frame = frame.borrow();
        let parent = frame.as_ref().and_then(|frame| frame.layers.last());
        (
            key,
            parent.map_or(0, |layer| std::rc::Rc::as_ptr(layer) as usize),
        )
    });
    if let Some(layer) = GO_BAND_LAYER_CACHE.with(|cache| cache.borrow().get(&layer_key).cloned()) {
        if layer.outside_rows(frame_band_rows(width, height)) {
            return true;
        }
        GO_BAND_FRAME.with(|frame| frame.borrow_mut().as_mut().unwrap().layers.push(layer));
        let drawn = draw(pixels);
        GO_BAND_FRAME.with(|frame| {
            frame.borrow_mut().as_mut().unwrap().layers.pop();
        });
        return drawn;
    }
    let cached = GO_BAND_LEAF_CACHE.with(|cache| cache.borrow().get(&key).cloned());
    let leaves = if let Some(leaves) = cached {
        leaves
    } else {
        GO_BAND_RECORD.with(|record| record.borrow_mut().push(Vec::new()));
        // Record-pass painters write nowhere that matters. A fresh mapped
        // layer keeps the pages they never touch out of resident memory.
        let mut scratch = ZeroedLayer::new(pixels.len());
        let recorded = draw(&mut scratch);
        drop(scratch);
        let mut leaves = GO_BAND_RECORD
            .with(|record| record.borrow_mut().pop())
            .unwrap_or_default();
        if !recorded {
            return false;
        }
        go_clip_leaves(
            &mut leaves,
            go_group_clip_rect(
                group,
                scale,
                svg_image_paths,
                raster_sources.native_canvas_size(width, height),
            ),
        );
        let leaves = std::rc::Rc::new(leaves);
        GO_BAND_LEAF_CACHE.with(|cache| cache.borrow_mut().insert(key, leaves.clone()));
        leaves
    };
    let canvas = (0, 0, width as isize, height as isize);
    let (windows, reference_bounds) = GO_BAND_FRAME.with(|frame| {
        let frame = frame.borrow();
        let parent = frame.as_ref().and_then(|frame| frame.layers.last());
        let parent_reference = parent.map_or(Some(canvas), |parent| parent.reference_bounds);
        let reference_bounds = go_node_bounds(&leaves, canvas)
            .zip(parent_reference)
            .and_then(|(bounds, parent)| go_rect_intersect(bounds, parent));
        let windows = (0..height)
            .step_by(GO_PNG_BAND_HEIGHT)
            .enumerate()
            .filter_map(|(band, top)| {
                let destination = match parent {
                    Some(parent) => parent.window(band)?,
                    None => (
                        0,
                        top as isize,
                        width as isize,
                        (top + GO_PNG_BAND_HEIGHT).min(height) as isize,
                    ),
                };
                go_effect_window(&leaves, canvas, destination).map(|window| (band, window))
            })
            .collect::<Vec<_>>();
        #[cfg(feature = "diagnostic-traces")]
        if raster_trace_pixel().is_some() {
            eprintln!(
                "d2-rs-go-effect id={:?} reference_bounds={reference_bounds:?} windows={windows:?}",
                group.id()
            );
            for (leaf, clip) in leaves.iter() {
                match leaf {
                    GoLeafBounds::Rect(l, t, r, b) => {
                        eprintln!("d2-rs-go-leaf rect=({l:.2},{t:.2},{r:.2},{b:.2}) clip={clip:?}")
                    }
                    GoLeafBounds::Stroke { segments, .. } => {
                        eprintln!(
                            "d2-rs-go-leaf stroke segments={} clip={clip:?}",
                            segments.len()
                        )
                    }
                }
            }
        }
        (windows, reference_bounds)
    });
    let layer = std::rc::Rc::new(GoLayer {
        windows,
        reference_bounds,
    });
    GO_BAND_LAYER_CACHE.with(|cache| cache.borrow_mut().insert(layer_key, layer.clone()));
    // Leaf inventories are build-only. The frozen reference and destination
    // windows replace them for every later band of this effect.
    GO_BAND_LEAF_CACHE.with(|cache| cache.borrow_mut().remove(&key));
    drop(leaves);
    if layer.outside_rows(frame_band_rows(width, height)) {
        return true;
    }
    GO_BAND_FRAME.with(|frame| {
        if let Some(frame) = frame.borrow_mut().as_mut() {
            frame.layers.push(layer);
        }
    });
    let drawn = draw(pixels);
    GO_BAND_FRAME.with(|frame| {
        if let Some(frame) = frame.borrow_mut().as_mut() {
            frame.layers.pop();
        }
    });
    drawn
}

#[allow(clippy::too_many_arguments)]
fn draw_group_impl_inner(
    tree: &Tree,
    group: &usvg::Group,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    text_outline: bool,
    ignore_masks: bool,
    inherited_reference_origin: Option<Point64>,
    inside_masked_layer: bool,
    effect_path_reference_origin: Option<Point64>,
    raster_sources: &RasterSourceMap<'_>,
    svg_image_paths: &SvgImagePathSources,
) -> bool {
    // Plain leaf geometry outside this band cannot contribute paint. Keep
    // filter subtrees intact: their off-band input may cast a visible shadow.
    if raster_sources.banded
        && group.filters().is_empty()
        && group
            .children()
            .iter()
            .all(|n| matches!(n, usvg::Node::Path(_) | usvg::Node::Text(_)))
    {
        let bounds = group.abs_stroke_bounding_box();
        let top = (f64::from(bounds.top()) * f64::from(scale)).floor() - 1.0;
        let bottom = (f64::from(bounds.bottom()) * f64::from(scale)).ceil() + 1.0;
        if bottom <= raster_sources.device_origin.y
            || top >= raster_sources.device_origin.y + height as f64
        {
            return true;
        }
    }
    // Group opacity requires an off-screen layer: multiplying each child
    // paint separately would change the result where children overlap. The
    // same is true for masks, clips, filters, isolation, and blend modes.
    let trivial_mask = group
        .mask()
        .is_some_and(|mask| is_opaque_white_mask(mask, group));
    // This validated viewport only transports native scene coordinates.
    // Go raster.go paints into the allocated frame, including its rounded
    // final pixel, without clipping rough ink to the logical viewBox.
    let trivial_clip = svg_image_paths.native_canvas_geometry(group.id()).is_some()
        || group
            .clip_path()
            .is_some_and(|clip| is_trivial_clip(clip, group));
    let group_bounds = group.abs_bounding_box();
    #[cfg(feature = "diagnostic-traces")]
    if let Some((trace_x, trace_y)) = raster_trace_pixel() {
        let left = f64::from(group_bounds.left()) * f64::from(scale);
        let top = f64::from(group_bounds.top()) * f64::from(scale);
        let right = f64::from(group_bounds.right()) * f64::from(scale);
        let bottom = f64::from(group_bounds.bottom()) * f64::from(scale);
        if (trace_x as f64) >= left
            && (trace_x as f64) < right
            && (trace_y as f64) >= top
            && (trace_y as f64) < bottom
        {
            eprintln!(
                "d2-rs-raster group id={:?} bounds=({left:.3},{top:.3},{right:.3},{bottom:.3}) mask={} trivial_mask={trivial_mask} clip={} trivial_clip={trivial_clip} ignore_masks={ignore_masks} inside_masked_layer={inside_masked_layer}",
                group.id(),
                group.mask().is_some(),
                group.clip_path().is_some(),
            );
        }
    }
    let document_viewport_clip = (group_bounds.width() - tree.size().width()).abs() < 1e-4
        && (group_bounds.height() - tree.size().height()).abs() < 1e-4;
    // D2's retained raster scene gives every vector-image viewport an effect
    // layer even when its generated clip covers the whole imported asset. Keep
    // that layer boundary: painting text directly onto the opaque parent skips
    // one premultiplied source-over quantization and changes antialiased glyph
    // pixels by one channel value.
    // A mask suppresses only its own second application; it does not suppress
    // clips on descendants such as rotated arrowhead viewports. Those clips
    // need the same retained effect layer as imported text viewports.
    let inherited_opacity = opacity;
    let group_opacity = group.opacity().get();
    // The validated native main viewport serializes ordinary scene ancestry.
    // Its parser-generated, covering clip must not create an effect layer:
    // Go paints these glyphs directly onto the parent (raster.go:1540-1558).
    // Real clips and unqualified viewport wrappers retain their layer below.
    if group.clip_path().is_some()
        && (!trivial_clip
            || (!document_viewport_clip
                && svg_image_paths.native_canvas_geometry(group.id()).is_none()
                && group_contains_text(group)))
    {
        if group_opacity != 1.0 || group.blend_mode() != usvg::BlendMode::Normal {
            return false;
        }
        return draw_clipped_group(
            tree,
            group,
            group.clip_path().expect("clip path exists"),
            pixels,
            width,
            height,
            scale,
            opacity,
            text_outline,
            ignore_masks,
            inside_masked_layer,
            effect_path_reference_origin,
            raster_sources,
            svg_image_paths,
        );
    }
    // Even an opaque white mask retains an off-screen RGBA layer in Go's
    // renderEffectNode. Painting straight onto the parent skips the layer's
    // byte quantization before source-over, changing antialiased edge pixels.
    if !ignore_masks && group.mask().is_some() {
        if group.blend_mode() != usvg::BlendMode::Normal
            && matches!(
                group.blend_mode(),
                usvg::BlendMode::Multiply
                    | usvg::BlendMode::Darken
                    | usvg::BlendMode::ColorBurn
                    | usvg::BlendMode::Overlay
                    | usvg::BlendMode::Lighten
            )
            && group.filters().is_empty()
            && group.clip_path().is_none()
            && !group.isolate()
        {
            return draw_masked_blend_group(
                tree,
                group,
                pixels,
                width,
                height,
                scale,
                inherited_opacity,
                text_outline,
                raster_sources,
                svg_image_paths,
            );
        }
        if group_opacity != 1.0 || group.blend_mode() != usvg::BlendMode::Normal {
            if !trivial_mask {
                return false;
            }
        } else {
            return draw_masked_group(
                tree,
                group,
                pixels,
                width,
                height,
                scale,
                opacity,
                text_outline,
                raster_sources,
                svg_image_paths,
            );
        }
    }
    // Static raster appendix badges carry a generated ID so the native path
    // can reproduce d2raster's three-box drop shadow. usvg does not apply the
    // CSS drop-shadow used by interactive SVG output, and resvg's Gaussian
    // primitive has different pixels from D2's quantized box-blur pipeline.
    if group.id().starts_with("d2-rs-static-appendix-icon-")
        && group.filters().is_empty()
        && !group.isolate()
        && group.opacity().get() == 1.0
        && group.blend_mode() == usvg::BlendMode::Normal
    {
        if raster_sources.banded {
            return false;
        }
        return draw_static_appendix_shadow(
            tree,
            group,
            pixels,
            width,
            height,
            scale,
            opacity,
            text_outline,
            ignore_masks,
            inherited_reference_origin,
            inside_masked_layer,
            effect_path_reference_origin,
            raster_sources,
            svg_image_paths,
            None,
        );
    }
    if let Some(transform) = raster_sources
        .native_shadow_transforms
        .get(&(group as *const usvg::Group))
        .copied()
    {
        if group.opacity().get() != 1.0
            || group.blend_mode() != usvg::BlendMode::Normal
            || group.clip_path().is_some()
            || group.mask().is_some()
            || group.isolate()
            || group.filters().len() != 1
        {
            return false;
        }
        if svg_image_paths.is_native_legend_shadow(group.id()) {
            if raster_sources.banded {
                return false;
            }
            return draw_static_appendix_shadow(
                tree,
                group,
                pixels,
                width,
                height,
                scale,
                opacity,
                text_outline,
                ignore_masks,
                inherited_reference_origin,
                inside_masked_layer,
                effect_path_reference_origin,
                raster_sources,
                svg_image_paths,
                Some((
                    transform,
                    svg_image_paths
                        .native_legend_shadow(group.id())
                        .expect("qualified source filter"),
                )),
            );
        }
        return draw_native_sharp_shadow(
            tree,
            group,
            pixels,
            width,
            height,
            scale,
            opacity,
            text_outline,
            inside_masked_layer,
            raster_sources,
            svg_image_paths,
            transform,
        );
    }
    // Source renderEffectNode: an isolated group without other effects is one
    // offscreen layer composited source-over, at its opacity, into its parent.
    if group.isolate()
        && group.blend_mode() == usvg::BlendMode::Normal
        && group.clip_path().is_none()
        && group.mask().is_none()
        && group.filters().is_empty()
    {
        return draw_effect_group(
            tree,
            group,
            pixels,
            width,
            height,
            scale,
            inherited_opacity,
            group_opacity,
            text_outline,
            inherited_reference_origin,
            inside_masked_layer,
            effect_path_reference_origin,
            raster_sources,
            svg_image_paths,
        );
    }
    if (group.clip_path().is_some() && !trivial_clip)
        || (!ignore_masks && group.mask().is_some() && !trivial_mask)
        || !group.filters().is_empty()
        || group.isolate()
    {
        #[cfg(feature = "diagnostic-traces")]
        if raster_trace_pixel().is_some() {
            eprintln!(
                "d2-rs-raster unsupported-group id={:?} filters={} clip={} mask={} isolate={}",
                group.id(),
                group.filters().len(),
                group.clip_path().is_some(),
                group.mask().is_some(),
                group.isolate(),
            );
        }
        return false;
    }
    if group_opacity != 1.0 || group.blend_mode() != usvg::BlendMode::Normal {
        if !matches!(
            group.blend_mode(),
            usvg::BlendMode::Normal
                | usvg::BlendMode::Multiply
                | usvg::BlendMode::Darken
                | usvg::BlendMode::ColorBurn
                | usvg::BlendMode::Overlay
                | usvg::BlendMode::Lighten
        ) {
            return false;
        }
        return draw_effect_group(
            tree,
            group,
            pixels,
            width,
            height,
            scale,
            inherited_opacity,
            group_opacity,
            text_outline,
            inherited_reference_origin,
            inside_masked_layer,
            effect_path_reference_origin,
            raster_sources,
            svg_image_paths,
        );
    }
    let opacity = inherited_opacity * group_opacity;
    let direct_text = group
        .children()
        .iter()
        .any(|child| matches!(child, usvg::Node::Text(_)));
    let mut reference_origin = if direct_text && group_contains_pattern_fill(group) {
        // Pattern fills are rendered as independent prepared resources in D2;
        // a text primitive beside a patterned shape retains the full-frame
        // origin rather than inheriting the shape's local raster boundary.
        Point64 { x: 0.0, y: 0.0 }
    } else if direct_text {
        group_reference_origin(group, scale)
    } else {
        inherited_reference_origin.unwrap_or_else(|| group_reference_origin(group, scale))
    };
    // D2's cropped mask layer reserves two device pixels of horizontal
    // clearance before submitting its child paths. Keep that shifted origin
    // on the layer itself so it also reaches paths nested in child groups;
    // resetting those descendants to (0, 0) rounds their large canvas
    // coordinates differently from the Go rasterizer.
    if ignore_masks && group.mask().is_some() {
        reference_origin.x -= 2.0;
    }
    draw_group_contents(
        tree,
        group,
        pixels,
        width,
        height,
        scale,
        opacity,
        text_outline,
        ignore_masks,
        reference_origin,
        inside_masked_layer || (ignore_masks && group.mask().is_some()),
        effect_path_reference_origin,
        raster_sources,
        svg_image_paths,
    )
}

// Source filter.go:473-520 and 918-1210: sample alpha at the translated
// device position, construct premultiplied NRGBA shadow, source-over original
// geometry, then source-over the filtered layer into its parent.
fn draw_native_sharp_shadow(
    tree: &Tree,
    group: &usvg::Group,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    text_outline: bool,
    inside_masked_layer: bool,
    raster_sources: &RasterSourceMap<'_>,
    sources: &SvgImagePathSources,
    transform: Affine64,
) -> bool {
    let _ = (tree, inside_masked_layer);
    // The shadow samples its source beyond the current band.
    let whole_frame = FrameBandScope::new(None);
    let origin = group_reference_origin(group, scale);
    let dx = transform.sx * 3.0 + transform.kx * 5.0;
    let dy = transform.ky * 3.0 + transform.sy * 5.0;
    let canvas_width = width;
    let canvas_height = if raster_sources.banded {
        raster_sources.canvas_height
    } else {
        height
    };
    let band_top = raster_sources.device_origin.y as usize;
    let band_bottom = band_top + height;
    let bounds = group.abs_stroke_bounding_box();
    let right = (f64::from(bounds.right()) * f64::from(scale) + dx.max(0.0) + 2.0)
        .ceil()
        .min(canvas_width as f64) as isize;
    let bottom = (f64::from(bounds.bottom()) * f64::from(scale) + dy.max(0.0) + 2.0)
        .ceil()
        .min(canvas_height as f64) as isize;
    // Go renderFilteredEffectNode renders into contentBounds, which can
    // extend outside the frame. Retain its signed origin so off-canvas ink
    // can contribute to a translated shadow before clipping the output.
    let crop_left = origin.x as isize;
    let crop_top = origin.y as isize;
    if crop_left >= right || crop_top >= bottom {
        return true;
    }
    // A streamed band pass writes only its own frame rows; this shadow and
    // its geometry write only rows crop_top..bottom.
    let (stream_top, stream_bottom) = whole_frame
        .band_rows(canvas_width, canvas_height)
        .filter(|_| !raster_sources.banded)
        .unwrap_or((0, usize::MAX));
    if bottom <= stream_top as isize || crop_top.max(0) as usize >= stream_bottom {
        return true;
    }
    let width = (right - crop_left) as usize;
    let height = (bottom - crop_top) as usize;
    let Some(bytes) = width.checked_mul(height).and_then(|n| n.checked_mul(4)) else {
        return false;
    };
    const MAX_SHADOW_REGION_BYTES: usize = 256 * 1024 * 1024;
    // Geometry can contain nested blend/opacity crops; budget the peak
    // simultaneous region layers in addition to the active band layers.
    let region_layers = 1 + effect_layer_units(group, false);
    if bytes
        .checked_mul(region_layers)
        .and_then(|bytes| bytes.checked_add(raster_sources.band_scratch_bytes))
        .is_none_or(|bytes| bytes > MAX_SHADOW_REGION_BYTES)
    {
        return false;
    }
    // filter.go:regionalFilters/shadowSampleBounds retains the unshifted
    // input plus every translated sample needed by this destination band.
    // Preserve the complete input's coordinate/rounding origin, but commit
    // pages and rasterize only the contributing rows.
    let destination_top = band_top.max(stream_top).max(crop_top.max(0) as usize);
    let destination_end = band_bottom
        .min(stream_bottom)
        .min((crop_top + height as isize).max(0) as usize);
    if destination_top >= destination_end {
        return true;
    }
    let input_top = (destination_top as f64 - crop_top as f64 - dy.max(0.0))
        .floor()
        .clamp(0.0, height as f64) as usize;
    let input_end = (destination_end as f64 - crop_top as f64 - dy.min(0.0) + 1.0)
        .ceil()
        .clamp(input_top as f64, height as f64) as usize;
    let input_region = LayerRegion::from_rects(
        [(0, input_top as isize, width as isize, input_end as isize)],
        width,
        height,
    );
    let mut geometry = ZeroedLayer::new(bytes);
    fn draw_geometry(
        group: &usvg::Group,
        geometry: &mut [u8],
        width: usize,
        height: usize,
        scale: f32,
        opacity: f32,
        text_outline: bool,
        origin: Point64,
        input_region: &LayerRegion,
        raster_sources: &RasterSourceMap<'_>,
        sources: &SvgImagePathSources,
    ) -> bool {
        group.children().iter().all(|child| match child {
            usvg::Node::Group(child)
                if child.opacity().get() == 1.0
                    && child.filters().is_empty()
                    && child.mask().is_none()
                    && child.clip_path().is_none()
                    && !child.isolate()
                    && child.blend_mode() == usvg::BlendMode::Normal =>
            {
                draw_geometry(
                    child,
                    geometry,
                    width,
                    height,
                    scale,
                    opacity,
                    text_outline,
                    origin,
                    input_region,
                    raster_sources,
                    sources,
                )
            }
            usvg::Node::Group(child)
                if child.filters().is_empty()
                    && child.mask().is_none()
                    && child.clip_path().is_none()
                    && !child.isolate()
                    && matches!(
                        child.blend_mode(),
                        usvg::BlendMode::Normal
                            | usvg::BlendMode::Multiply
                            | usvg::BlendMode::Darken
                            | usvg::BlendMode::ColorBurn
                            | usvg::BlendMode::Overlay
                            | usvg::BlendMode::Lighten
                    ) =>
            {
                // Source filters the complete prepared geometry, including
                // streak blend children. Retain the crop's device origin for
                // path/pattern sampling and translate only compositing bounds.
                let mut layer = ZeroedLayer::new(geometry.len());
                if !draw_geometry(
                    child,
                    &mut layer,
                    width,
                    height,
                    scale,
                    opacity,
                    text_outline,
                    origin,
                    input_region,
                    raster_sources,
                    sources,
                ) {
                    return false;
                }
                let bounds = child.abs_layer_bounding_box();
                let Some(crop_bounds) = resvg::tiny_skia::Rect::from_xywh(
                    bounds.x() - (origin.x / f64::from(scale)) as f32,
                    bounds.y() - (origin.y / f64::from(scale)) as f32,
                    bounds.width(),
                    bounds.height(),
                ) else {
                    return false;
                };
                composite_effect_group(
                    geometry,
                    &layer,
                    width,
                    height,
                    crop_bounds,
                    input_region,
                    scale,
                    sources.exact_effect_opacity(child.id(), child.opacity().get()),
                    child.blend_mode(),
                );
                true
            }
            usvg::Node::Path(path) if !path.is_visible() => true,
            usvg::Node::Path(path) => draw_path_with_transform(
                path,
                geometry,
                width,
                height,
                scale,
                opacity,
                text_outline,
                None,
                Point64 { x: 0.0, y: 0.0 },
                Point64 { x: 0.0, y: 0.0 },
                path_uses_analytic_paint(path),
                raster_sources,
                origin,
                sources.raw_path(path.id()),
            ),
            _ => false,
        })
    }
    // Source filter input is the prepared geometry rectangle, not the frame.
    // Keep exact device transforms while rendering into this bounded layer.
    let geometry_rows = FrameBandScope::new(Some((width, height, input_top, input_end)));
    if !draw_geometry(
        group,
        &mut geometry,
        width,
        height,
        scale,
        opacity,
        text_outline,
        origin,
        &input_region,
        raster_sources,
        sources,
    ) {
        return false;
    }
    drop(geometry_rows);
    let alpha_at = |x: isize, y: isize| -> f64 {
        if x < 0 || y < 0 || x as usize >= width || y as usize >= height {
            0.0
        } else {
            f64::from(geometry[(y as usize * width + x as usize) * 4 + 3])
        }
    };
    // Restrict sampling to the actual alpha support and its translated union.
    // Bilinear sampling can reach one adjacent pixel, so keep a full-pixel
    // border. This preserves complete coverage without scanning the frame for
    // every filter sample on large diagrams with many small shadowed shapes.
    // Any nonzero byte bounds the support from outside: pixels it adds have
    // zero alpha and zero shadow, which composite nothing.
    let row_bytes = width * 4;
    let Some((min_x, min_y, max_x, max_y)) = nonzero_pixel_bounds(
        &geometry[input_top * row_bytes..input_end * row_bytes],
        width,
    ) else {
        return true;
    };
    let (min_y, max_y) = (min_y + input_top, max_y + input_top);
    let left = (min_x as f64).min(min_x as f64 + dx).floor().max(0.0) as usize;
    let top = (min_y as f64).min(min_y as f64 + dy).floor().max(0.0) as usize;
    let right = ((max_x as f64).max(max_x as f64 + dx).ceil() + 2.0).min(width as f64) as usize;
    let bottom = ((max_y as f64).max(max_y as f64 + dy).ceil() + 2.0).min(height as f64) as usize;
    let local_row = |row: usize| row.saturating_add_signed(-crop_top);
    let first_row = top.max(local_row(band_top)).max(local_row(stream_top));
    let end_row = bottom
        .min(local_row(band_bottom))
        .min(local_row(stream_bottom));
    let left = left.max((-crop_left).max(0) as usize);
    // The shadow's premultiplied color for each sampled source alpha.
    let shadow_colors: [[u8; 4]; 256] = std::array::from_fn(|alpha| {
        let shadow_alpha = (alpha as u32 * 102 + 127) / 255;
        [
            ((61 * shadow_alpha + 127) / 255) as u8,
            ((69 * shadow_alpha + 127) / 255) as u8,
            ((116 * shadow_alpha + 127) / 255) as u8,
            shadow_alpha as u8,
        ]
    });
    // A whole-pixel offset samples each source pixel exactly: the bilinear
    // weights are zero, so the interpolation returns that pixel's alpha.
    let whole_offset = (dx.fract() == 0.0 && dy.fract() == 0.0)
        .then_some((dx as isize, dy as isize))
        .filter(|&(x, y)| x as f64 == dx && y as f64 == dy);
    for y in first_row..end_row {
        for x in left..right {
            let alpha = if let Some((offset_x, offset_y)) = whole_offset {
                alpha_at(x as isize - offset_x, y as isize - offset_y) as u32
            } else {
                let sx = x as f64 - dx;
                let sy = y as f64 - dy;
                let mut alpha = 0_u32;
                if sx >= -1.0 && sy >= -1.0 && sx < width as f64 && sy < height as f64 {
                    let x0 = sx.floor() as isize;
                    let y0 = sy.floor() as isize;
                    let fx = sx - sx.floor();
                    let fy = sy - sy.floor();
                    let top = alpha_at(x0, y0) + (alpha_at(x0 + 1, y0) - alpha_at(x0, y0)) * fx;
                    let bottom = alpha_at(x0, y0 + 1)
                        + (alpha_at(x0 + 1, y0 + 1) - alpha_at(x0, y0 + 1)) * fx;
                    alpha = (top + (bottom - top) * fy).round().clamp(0.0, 255.0) as u32;
                }
                alpha
            };
            let index = (y * width + x) * 4;
            let source: [u8; 4] = geometry[index..index + 4].try_into().unwrap();
            // Neither the shadow nor the source composites anything here.
            if alpha == 0 && source[3] == 0 {
                continue;
            }
            let mut filtered = shadow_colors[alpha as usize];
            blend_premultiplied_pixel(&mut filtered, source);
            let destination_index = (((y as isize + crop_top) as usize - band_top) * canvas_width
                + (x as isize + crop_left) as usize)
                * 4;
            blend_premultiplied_pixel(
                &mut pixels[destination_index..destination_index + 4],
                filtered,
            );
        }
    }
    true
}

/// The bounds (first and last column and row) of the pixels of a
/// `width`-pixel RGBA surface with any nonzero byte, scanned eight bytes at a
/// time; None when the surface is all zero.
fn nonzero_pixel_bounds(surface: &[u8], width: usize) -> Option<(usize, usize, usize, usize)> {
    let row_bytes = width * 4;
    let word = |bytes: &[u8]| u64::from_le_bytes(bytes.try_into().unwrap());
    let first_byte = |row: &[u8]| -> Option<usize> {
        let (words, tail) = row.as_chunks::<8>();
        for (index, bytes) in words.iter().enumerate() {
            let value = word(bytes);
            if value != 0 {
                return Some(index * 8 + value.trailing_zeros() as usize / 8);
            }
        }
        let start = words.len() * 8;
        tail.iter()
            .position(|&byte| byte != 0)
            .map(|index| start + index)
    };
    let last_byte = |row: &[u8]| -> usize {
        let (head, words) = row.as_rchunks::<8>();
        for (index, bytes) in words.iter().enumerate().rev() {
            let value = word(bytes);
            if value != 0 {
                return head.len() + index * 8 + 7 - value.leading_zeros() as usize / 8;
            }
        }
        head.iter().rposition(|&byte| byte != 0).unwrap()
    };
    let mut bounds: Option<(usize, usize, usize, usize)> = None;
    for (y, row) in surface.chunks_exact(row_bytes).enumerate() {
        let Some(first) = first_byte(row) else {
            continue;
        };
        let (first, last) = (first / 4, last_byte(row) / 4);
        bounds = Some(match bounds {
            None => (first, y, last, y),
            Some((x0, y0, x1, _)) => (x0.min(first), y0, x1.max(last), y),
        });
    }
    bounds
}

fn draw_static_appendix_shadow(
    tree: &Tree,
    group: &usvg::Group,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    text_outline: bool,
    ignore_masks: bool,
    inherited_reference_origin: Option<Point64>,
    inside_masked_layer: bool,
    effect_path_reference_origin: Option<Point64>,
    raster_sources: &RasterSourceMap<'_>,
    svg_image_paths: &SvgImagePathSources,
    native_legend: Option<(Affine64, crate::svg_precision::NativeDropShadow)>,
) -> bool {
    // The blurred shadow samples its source beyond the current band.
    let _whole_frame = FrameBandScope::new(None);
    let (sigma_x, sigma_y, offset_x, offset_y, color): (f64, f64, f64, f64, [u8; 4]) =
        if let Some((t, filter)) = native_legend {
            let bounds = group.abs_stroke_bounding_box();
            if f64::from(bounds.left()) * f64::from(scale) < 0.0
                || f64::from(bounds.top()) * f64::from(scale) < 0.0
                || f64::from(bounds.right()) * f64::from(scale) > width as f64
                || f64::from(bounds.bottom()) * f64::from(scale) > height as f64
                || pixels.len() > 256 * 1024 * 1024
            {
                return false;
            }
            let dx = t.sx * filter.offset[0] + t.kx * filter.offset[1];
            let dy = t.ky * filter.offset[0] + t.sy * filter.offset[1];
            if !dx.is_finite()
                || !dy.is_finite()
                || dx.abs() > isize::MAX as f64 / 2.0
                || dy.abs() > isize::MAX as f64 / 2.0
            {
                return false;
            }
            (
                (t.sx * filter.sigma[0]).hypot(t.kx * filter.sigma[1]),
                (t.ky * filter.sigma[0]).hypot(t.sy * filter.sigma[1]),
                dx,
                dy,
                filter.color,
            )
        } else {
            (
                32.0 * f64::from(scale),
                32.0 * f64::from(scale),
                0.0,
                0.0,
                [31, 36, 58, 26],
            )
        };
    let reference_origin = if group
        .children()
        .iter()
        .any(|child| matches!(child, usvg::Node::Text(_)))
    {
        group_reference_origin(group, scale)
    } else {
        inherited_reference_origin.unwrap_or_else(|| group_reference_origin(group, scale))
    };
    // bands.go setBandReferenceBounds resets a filtered node's primitive
    // origins to contentBounds, before the blur/offset expands those bounds.
    // The full-frame origin changes float32 stroke coverage at half pixels.
    let (reference_origin, effect_path_reference_origin) = if native_legend.is_some() {
        let origin = native_prepared_bounds(tree, group, None, scale, svg_image_paths)
            .map(TextPaintBounds::origin)
            .unwrap_or_else(|| group_reference_origin(group, scale));
        (origin, Some(origin))
    } else {
        (reference_origin, effect_path_reference_origin)
    };
    // filter.go renders the complete prepared input before applying the
    // shadow. A badge's stroke can extend beyond the page (in particular the
    // appendix circle at x=16); clipping it first loses blur contributions.
    let bounds = group.abs_stroke_bounding_box();
    let (source_left, source_top, source_right, source_bottom) = if native_legend.is_none() {
        (
            ((f64::from(bounds.left()) * f64::from(scale))
                .floor()
                .min(0.0) as isize)
                .saturating_sub(1),
            ((f64::from(bounds.top()) * f64::from(scale))
                .floor()
                .min(0.0) as isize)
                .saturating_sub(1),
            ((f64::from(bounds.right()) * f64::from(scale))
                .ceil()
                .max(width as f64) as isize)
                .saturating_add(1),
            ((f64::from(bounds.bottom()) * f64::from(scale))
                .ceil()
                .max(height as f64) as isize)
                .saturating_add(1),
        )
    } else {
        (0, 0, width as isize, height as isize)
    };
    let (Some(source_width), Some(source_height)) = (
        source_right
            .checked_sub(source_left)
            .and_then(|n| usize::try_from(n).ok()),
        source_bottom
            .checked_sub(source_top)
            .and_then(|n| usize::try_from(n).ok()),
    ) else {
        return false;
    };
    let Some(source_bytes) = source_width
        .checked_mul(source_height)
        .and_then(|n| n.checked_mul(4))
    else {
        return false;
    };
    if source_bytes > 256 * 1024 * 1024 {
        return false;
    }
    // A band pass writes only its own rows. The shadow and badge stay
    // within the badge bounds expanded by the blur support and offset, with
    // a margin for stroke and glyph ink beyond usvg's bounding box.
    if let Some((band_top, band_bottom)) = _whole_frame.band_rows(width, height) {
        let margin = 32.0 * f64::from(scale) + 8.0;
        let reach = (3.0 * sigma_y).ceil() + 3.0 + offset_y.abs() + margin;
        let first = (f64::from(bounds.top()) * f64::from(scale) - reach).floor();
        let end = (f64::from(bounds.bottom()) * f64::from(scale) + reach).ceil();
        // Keep the full work when no band would ever reach it, so a failure
        // below still surfaces exactly as in a whole-frame render.
        let reaches_frame = end > 0.0 && first < height as f64;
        if reaches_frame && (end <= band_top as f64 || first >= band_bottom as f64) {
            return true;
        }
    }
    // The finished shadow and badge of one appendix icon, kept while the
    // band passes that cross it are drawn: the blur runs once per icon.
    let band = _whole_frame.band_rows(width, height);
    let shadow_key: AppendixShadowKey = (
        group as *const usvg::Group as usize,
        [
            scale.to_bits(),
            opacity.to_bits(),
            u32::from(text_outline),
            u32::from(ignore_masks),
            u32::from(inside_masked_layer),
        ],
        [
            reference_origin.x.to_bits(),
            reference_origin.y.to_bits(),
            effect_path_reference_origin.map_or(u64::MAX, |origin| origin.x.to_bits()),
            effect_path_reference_origin.map_or(u64::MAX, |origin| origin.y.to_bits()),
        ],
        (width, height),
    );
    // A band record pass paints nothing, so it neither uses nor keeps one.
    let cacheable = band.is_some() && !go_band_recording();
    let cached = cacheable.then_some(()).and_then(|()| {
        TEXT_MEMO.with(|memo| {
            memo.borrow()
                .as_ref()
                .and_then(|memo| memo.appendix.get(&shadow_key).cloned())
        })
    });
    let shadow = match cached {
        Some(shadow) => shadow,
        None => {
            let Some(shadow) = render_appendix_shadow(
                tree,
                group,
                width,
                height,
                scale,
                opacity,
                text_outline,
                ignore_masks,
                reference_origin,
                inside_masked_layer,
                effect_path_reference_origin,
                raster_sources,
                svg_image_paths,
                (
                    source_left,
                    source_top,
                    source_width,
                    source_height,
                    source_bytes,
                ),
                (sigma_x, sigma_y, offset_x, offset_y, color),
                native_legend.is_none(),
            ) else {
                return false;
            };
            let shadow = shadow.map(std::rc::Rc::new);
            if cacheable {
                text_memo_store(|memo| &mut memo.appendix, shadow_key, shadow.clone());
            }
            shadow
        }
    };
    let Some(shadow) = shadow else {
        return true;
    };
    let (first_row, end_row) = band.unwrap_or((0, height));
    let shadow_width = shadow.right - shadow.left;
    for y in shadow.top.max(first_row)..shadow.bottom.min(end_row) {
        let row = (y - shadow.top) * shadow_width;
        for x in shadow.left..shadow.right {
            let start = (row + x - shadow.left) * 4;
            let filtered = &shadow.pixels[start..start + 4];
            let pixel_start = (y * width + x) * 4;
            blend_premultiplied_pixel(
                &mut pixels[pixel_start..pixel_start + 4],
                [filtered[0], filtered[1], filtered[2], filtered[3]],
            );
        }
    }
    // Bands are drawn top to bottom: no later pass reaches this icon.
    if band.is_some_and(|(_, band_bottom)| shadow.bottom <= band_bottom) {
        TEXT_MEMO.with(|memo| {
            if let Some(memo) = memo.borrow_mut().as_mut() {
                memo.appendix.remove(&shadow_key);
            }
        });
    }
    true
}

/// An appendix icon's composited shadow and badge over frame pixels
/// `left..right` x `top..bottom`, premultiplied RGBA.
struct AppendixShadow {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
    pixels: Vec<u8>,
}

type AppendixShadowKey = (usize, [u32; 5], [u64; 4], (usize, usize));

/// Render the badge source, blur its alpha and composite the badge over its
/// shadow (draw_static_appendix_shadow). None on unsupported content; an
/// inner None when the badge paints nothing.
fn render_appendix_shadow(
    tree: &Tree,
    group: &usvg::Group,
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    text_outline: bool,
    ignore_masks: bool,
    reference_origin: Point64,
    inside_masked_layer: bool,
    effect_path_reference_origin: Option<Point64>,
    raster_sources: &RasterSourceMap<'_>,
    svg_image_paths: &SvgImagePathSources,
    (source_left, source_top, source_width, source_height, source_bytes): (
        isize,
        isize,
        usize,
        usize,
        usize,
    ),
    (sigma_x, sigma_y, offset_x, offset_y, color): (f64, f64, f64, f64, [u8; 4]),
    region_origin: bool,
) -> Option<Option<AppendixShadow>> {
    let region_sources = region_origin.then(|| {
        let mut region = raster_sources.clone();
        region.device_origin = Point64 {
            x: source_left as f64,
            y: source_top as f64,
        };
        region
    });
    // The source spans at least the whole frame but receives one badge: map
    // it lazily and bound what its content paints.
    let mut source = ZeroedLayer::new(source_bytes);
    let source_scope = LayerPaintScope::begin_contents(tree, group, &source);
    if !draw_group_contents(
        tree,
        group,
        &mut source,
        source_width,
        source_height,
        scale,
        opacity,
        text_outline,
        ignore_masks,
        reference_origin,
        inside_masked_layer,
        effect_path_reference_origin,
        region_sources.as_ref().unwrap_or(raster_sources),
        svg_image_paths,
    ) {
        return None;
    }

    let radii = |sigma: f64| -> Option<[usize; 3]> {
        let support = (3.0 * sigma).ceil();
        if !support.is_finite() || support > (usize::MAX / 6) as f64 {
            return None;
        }
        let support = support as usize;
        let base = support / 3;
        let remainder = support % 3;
        Some([
            base + usize::from(remainder > 0),
            base + usize::from(remainder > 1),
            base,
        ])
    };
    let (x_radii, y_radii) = (radii(sigma_x)?, radii(sigma_y)?);
    // The filter's alpha can leave the canvas and contribute to a later
    // pass back inside it. Retain occupied source bounds and expand each pass;
    // clipping intermediate layers to the page changes the integer blur.
    let painted = source_scope.finish(source_width, source_height);
    let Some(mut alpha) = occupied_alpha_region_within(&source, source_width, &painted) else {
        return Some(None);
    };
    alpha.left += source_left;
    alpha.top += source_top;
    let source_bounds = (
        alpha.left,
        alpha.top,
        alpha.left + alpha.width as isize,
        alpha.top + alpha.height as isize,
    );
    for index in 0..3 {
        let horizontal = box_blur_alpha_expanded(&alpha, x_radii[index], true)?;
        alpha = box_blur_alpha_expanded(&horizontal, y_radii[index], false)?;
    }

    // Go paints the premultiplied shadow into a transparent filter layer,
    // composites the original badge over it, then composites that result onto
    // the page. Preserve those two byte-rounded source-over operations.
    // Include the original source and the fractional shadow support. Sampling
    // follows filter.go sampleShadowAlphaImage, including its single byte round.
    let left = ((alpha.left as f64 + offset_x).floor() as isize)
        .min(source_bounds.0)
        .max(0) as usize;
    let top = ((alpha.top as f64 + offset_y).floor() as isize)
        .min(source_bounds.1)
        .max(0) as usize;
    let right = (((alpha.left + alpha.width as isize) as f64 + offset_x).ceil() as isize)
        .max(source_bounds.2)
        .min(width as isize)
        .max(0) as usize;
    let bottom = (((alpha.top + alpha.height as isize) as f64 + offset_y).ceil() as isize)
        .max(source_bounds.3)
        .min(height as isize)
        .max(0) as usize;
    let (right, bottom) = (right.max(left), bottom.max(top));
    let mut pixels = vec![0_u8; (right - left) * (bottom - top) * 4];
    for y in top..bottom {
        for x in left..right {
            let sampled =
                sample_shadow_alpha_region(&alpha, x as f64 - offset_x, y as f64 - offset_y);
            let source_start = ((y as isize - source_top) as usize * source_width
                + (x as isize - source_left) as usize)
                * 4;
            let source_pixel = &source[source_start..source_start + 4];
            let shadow_alpha = (u32::from(sampled) * u32::from(color[3]) + 127) / 255;
            // Both transparent: compositing nothing leaves the page as is.
            if shadow_alpha == 0 && source_pixel[3] == 0 {
                continue;
            }
            let mut filtered = [
                ((u32::from(color[0]) * shadow_alpha + 127) / 255) as u8,
                ((u32::from(color[1]) * shadow_alpha + 127) / 255) as u8,
                ((u32::from(color[2]) * shadow_alpha + 127) / 255) as u8,
                shadow_alpha as u8,
            ];
            blend_premultiplied_pixel(
                &mut filtered,
                [
                    source_pixel[0],
                    source_pixel[1],
                    source_pixel[2],
                    source_pixel[3],
                ],
            );
            let start = ((y - top) * (right - left) + x - left) * 4;
            pixels[start..start + 4].copy_from_slice(&filtered);
        }
    }
    Some(Some(AppendixShadow {
        left,
        top,
        right,
        bottom,
        pixels,
    }))
}

#[derive(Clone)]
struct AlphaRegion {
    left: isize,
    top: isize,
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

/// Original filter.go sampleShadowAlphaImage/interpolateShadowAlpha.
fn sample_shadow_alpha_region(alpha: &AlphaRegion, x: f64, y: f64) -> u8 {
    let right = alpha.left as f64 + alpha.width as f64;
    let bottom = alpha.top as f64 + alpha.height as f64;
    if !x.is_finite()
        || !y.is_finite()
        || x < alpha.left as f64 - 1.0
        || x >= right
        || y < alpha.top as f64 - 1.0
        || y >= bottom
    {
        return 0;
    }
    let x0 = x.floor() as isize;
    let y0 = y.floor() as isize;
    let fx = x - x0 as f64;
    let fy = y - y0 as f64;
    let at = |px: isize, py: isize| -> f64 {
        let px = px - alpha.left;
        let py = py - alpha.top;
        if px < 0 || py < 0 || px >= alpha.width as isize || py >= alpha.height as isize {
            0.0
        } else {
            f64::from(alpha.pixels[py as usize * alpha.width + px as usize])
        }
    };
    if fx == 0.0 && fy == 0.0 {
        return at(x0, y0) as u8;
    }
    let a00 = at(x0, y0);
    let a01 = at(x0, y0 + 1);
    let top = (at(x0 + 1, y0) - a00).mul_add(fx, a00);
    let bottom = (at(x0 + 1, y0 + 1) - a01).mul_add(fx, a01);
    (bottom - top).mul_add(fy, top).round().clamp(0.0, 255.0) as u8
}

/// `occupied_alpha_region` of a layer with no alpha outside `region`.
fn occupied_alpha_region_within(
    source: &[u8],
    width: usize,
    region: &LayerRegion,
) -> Option<AlphaRegion> {
    let (mut left, mut top, mut right, mut bottom) = (usize::MAX, usize::MAX, 0, 0);
    for (y, x0, x1, _) in region.rows_in(0, usize::MAX) {
        for x in x0..x1 {
            if source[(y * width + x) * 4 + 3] != 0 {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    if left == usize::MAX {
        return None;
    }
    let mut pixels = Vec::with_capacity((right - left) * (bottom - top));
    for y in top..bottom {
        for x in left..right {
            pixels.push(source[(y * width + x) * 4 + 3]);
        }
    }
    Some(AlphaRegion {
        left: left as isize,
        top: top as isize,
        width: right - left,
        height: bottom - top,
        pixels,
    })
}

#[cfg(test)]
fn occupied_alpha_region(source: &[u8], width: usize, height: usize) -> Option<AlphaRegion> {
    let mut left = width;
    let mut top = height;
    let mut right = 0;
    let mut bottom = 0;
    for (index, pixel) in source.chunks_exact(4).enumerate() {
        if pixel[3] != 0 {
            let x = index % width;
            let y = index / width;
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    if left == width {
        return None;
    }
    let mut pixels = Vec::with_capacity((right - left) * (bottom - top));
    for y in top..bottom {
        for x in left..right {
            pixels.push(source[(y * width + x) * 4 + 3]);
        }
    }
    Some(AlphaRegion {
        left: left as isize,
        top: top as isize,
        width: right - left,
        height: bottom - top,
        pixels,
    })
}

fn box_blur_alpha_expanded(
    source: &AlphaRegion,
    radius: usize,
    horizontal: bool,
) -> Option<AlphaRegion> {
    if radius == 0 {
        return Some(source.clone());
    }
    let twice_radius = radius.checked_mul(2)?;
    let window = twice_radius.checked_add(1)?;
    let width = if horizontal {
        source.width.checked_add(twice_radius)?
    } else {
        source.width
    };
    let height = if horizontal {
        source.height
    } else {
        source.height.checked_add(twice_radius)?
    };
    let radius_offset = isize::try_from(radius).ok()?;
    let left = if horizontal {
        source.left.checked_sub(radius_offset)?
    } else {
        source.left
    };
    let top = if horizontal {
        source.top
    } else {
        source.top.checked_sub(radius_offset)?
    };
    left.checked_add(isize::try_from(width).ok()?)?;
    top.checked_add(isize::try_from(height).ok()?)?;
    let mut pixels = vec![0; width.checked_mul(height)?];
    let half_window = (window / 2) as u64;
    let denominator = window as u64;
    // A window sum is at most 255 * window: tabulate its rounded average
    // rather than dividing for every output sample.
    let max_sum = window.checked_mul(255)?;
    let averages = (max_sum <= 1 << 20 && pixels.len() > max_sum).then(|| {
        (0..=max_sum as u64)
            .map(|sum| ((sum + half_window) / denominator) as u8)
            .collect::<Vec<_>>()
    });
    let average = |sum: u64| match &averages {
        Some(averages) => averages[sum as usize],
        None => ((sum + half_window) / denominator) as u8,
    };
    if horizontal {
        for y in 0..height {
            let row = y * source.width;
            let mut sum = u64::from(source.pixels[row]);
            for x in 0..width {
                pixels[y * width + x] = average(sum);
                if x >= twice_radius {
                    sum -= u64::from(source.pixels[row + x - twice_radius]);
                }
                if x + 1 < source.width {
                    sum += u64::from(source.pixels[row + x + 1]);
                }
            }
        }
    } else {
        // The same per-column running sums, advanced a row at a time so the
        // passes read and write contiguous rows.
        let mut sums = source.pixels[..width]
            .iter()
            .map(|&value| u64::from(value))
            .collect::<Vec<_>>();
        for y in 0..height {
            for (output, &sum) in pixels[y * width..(y + 1) * width].iter_mut().zip(&sums) {
                *output = average(sum);
            }
            if y >= twice_radius {
                let row = &source.pixels[(y - twice_radius) * source.width..][..width];
                for (sum, &value) in sums.iter_mut().zip(row) {
                    *sum -= u64::from(value);
                }
            }
            if y + 1 < source.height {
                let row = &source.pixels[(y + 1) * source.width..][..width];
                for (sum, &value) in sums.iter_mut().zip(row) {
                    *sum += u64::from(value);
                }
            }
        }
    }
    Some(AlphaRegion {
        left,
        top,
        width,
        height,
        pixels,
    })
}

fn group_contains_text(group: &usvg::Group) -> bool {
    group.children().iter().any(|child| match child {
        usvg::Node::Text(_) => true,
        usvg::Node::Group(group) => group_contains_text(group),
        usvg::Node::Path(_) | usvg::Node::Image(_) => false,
    })
}

fn group_contains_pattern_fill(group: &usvg::Group) -> bool {
    group.children().iter().any(|child| match child {
        usvg::Node::Group(group) => group_contains_pattern_fill(group),
        usvg::Node::Path(path) => path
            .fill()
            .is_some_and(|fill| matches!(fill.paint(), Paint::Pattern(_))),
        usvg::Node::Image(_) | usvg::Node::Text(_) => false,
    })
}

fn draw_clipped_group(
    tree: &Tree,
    group: &usvg::Group,
    clip: &usvg::ClipPath,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    text_outline: bool,
    ignore_masks: bool,
    inside_masked_layer: bool,
    effect_path_reference_origin: Option<Point64>,
    raster_sources: &RasterSourceMap<'_>,
    svg_image_paths: &SvgImagePathSources,
) -> bool {
    let mut layer = take_effect_layer(pixels.len());
    let scope = LayerPaintScope::begin(tree, group, &layer);
    #[cfg(feature = "diagnostic-traces")]
    if raster_trace_pixel().is_some() {
        eprintln!(
            "d2-rs-raster clip-transform group_transform={:?} clip_transform={:?} root_transform={:?} root_bounds={:?}",
            group.abs_transform(),
            clip.transform(),
            clip.root().abs_transform(),
            clip.root().abs_bounding_box(),
        );
    }
    let effect_path_reference_origin = raster_sources
        .paint_reference_contexts
        .get(group.id())
        .and_then(|context| context.paint_bounds)
        .map(TextPaintBounds::origin)
        .or(effect_path_reference_origin);
    let reference_origin = if group_visible_path_count(group) >= 2 {
        group_reference_origin(group, scale)
    } else {
        Point64 { x: 0.0, y: 0.0 }
    };
    if !draw_group_contents(
        tree,
        group,
        &mut layer,
        width,
        height,
        scale,
        opacity,
        text_outline,
        ignore_masks,
        reference_origin,
        inside_masked_layer,
        effect_path_reference_origin,
        raster_sources,
        svg_image_paths,
    ) {
        return false;
    }
    let region = scope.finish(width, height);
    debug_assert_layer_in_region(&layer, width, &region);
    let native_markdown = svg_image_paths.native_markdown_viewport(group.id()).zip(
        raster_sources
            .native_shadow_transforms
            .get(&(group as *const usvg::Group)),
    );
    let clip_alpha = if let Some(([_, _, clip_width, clip_height], transform)) = native_markdown {
        // buildMarkdownLabel's boxClip shares the complete f64 placement
        // with its background/text and rounds relative to the effect layer
        // (bands.go setBandReferenceBounds), not to the full canvas.
        let mut rasterizer = Rasterizer::new(width, height, 0.375);
        let reference = effect_path_reference_origin.unwrap_or_default();
        rasterizer.set_reference_origin(Point64 {
            x: reference.x - raster_sources.device_origin.x,
            y: reference.y - raster_sources.device_origin.y,
        });
        let mut transform = *transform;
        transform.tx -= raster_sources.device_origin.x;
        transform.ty -= raster_sources.device_origin.y;
        let points = [
            Point64 { x: 0.0, y: 0.0 },
            Point64 {
                x: clip_width,
                y: 0.0,
            },
            Point64 {
                x: clip_width,
                y: clip_height,
            },
            Point64 {
                x: 0.0,
                y: clip_height,
            },
        ]
        .map(|point| map_point(point, transform));
        rasterizer.add_fill_polyline(&points, true);
        rasterizer.set_output_region(region.clone());
        let mut coverage = ZeroedLayer::new(region.len * 4);
        let recording = go_band_recording();
        if recording {
            GO_BAND_RECORD.with(|record| record.borrow_mut().push(Vec::new()));
        }
        rasterizer.paint(&mut coverage, [255; 4]);
        if recording {
            GO_BAND_RECORD.with(|record| record.borrow_mut().pop());
        }
        let mut alpha = ZeroedLayer::new(region.len);
        for (alpha, pixel) in alpha.iter_mut().zip(coverage.chunks_exact(4)) {
            *alpha = pixel[3];
        }
        Some(alpha)
    } else {
        render_clip_alpha(
            clip,
            group.abs_transform(),
            width,
            height,
            &region,
            scale,
            raster_sources.device_origin,
            svg_image_paths,
        )
    };
    let Some(clip_alpha) = clip_alpha else {
        return false;
    };
    #[cfg(feature = "diagnostic-traces")]
    let trace_pixel = raster_trace_pixel();
    // Pixels outside the region are transparent: clipping keeps them so,
    // and they composite nothing.
    for (y, x0, x1, offset) in region.rows_in(0, height) {
        let row = y * width + x0..y * width + x1;
        let alpha_row = &clip_alpha[offset..offset + (x1 - x0)];
        for (index, &alpha) in row.zip(alpha_row) {
            let pixel = &mut layer[index * 4..index * 4 + 4];
            #[cfg(feature = "diagnostic-traces")]
            let trace_this_pixel = trace_pixel == Some((index % width, index / width));
            #[cfg(feature = "diagnostic-traces")]
            let before = trace_this_pixel.then(|| pixel.to_vec());
            scale_premultiplied(pixel, alpha);
            #[cfg(feature = "diagnostic-traces")]
            if let Some(before) = before {
                eprintln!(
                    "d2-rs-raster clip-composite x={} y={} alpha={alpha} layer_before={before:?} layer_after={:?} group_bounds=({:.3},{:.3},{:.3},{:.3})",
                    index % width,
                    index / width,
                    pixel,
                    group.abs_bounding_box().left(),
                    group.abs_bounding_box().top(),
                    group.abs_bounding_box().right(),
                    group.abs_bounding_box().bottom(),
                );
            }
            if pixel[3] != 0 {
                blend_premultiplied_pixel(
                    &mut pixels[index * 4..index * 4 + 4],
                    [pixel[0], pixel[1], pixel[2], pixel[3]],
                );
            }
        }
    }
    note_painted(pixels, &region);
    recycle_effect_layer(layer, width, height, region);
    true
}

fn group_visible_path_count(group: &usvg::Group) -> usize {
    group
        .children()
        .iter()
        .map(|child| match child {
            usvg::Node::Group(group) => group_visible_path_count(group),
            usvg::Node::Path(path) if path.is_visible() => 1,
            usvg::Node::Path(_) | usvg::Node::Image(_) | usvg::Node::Text(_) => 0,
        })
        .sum()
}

fn scale_premultiplied(pixel: &mut [u8], alpha: u8) {
    match alpha {
        0 => pixel.fill(0),
        255 => {}
        alpha => {
            for channel in pixel.iter_mut() {
                *channel = ((u32::from(*channel) * u32::from(alpha) + 127) / 255) as u8;
            }
        }
    }
}

/// Clip coverage for the pixels of `region`, in its compact layout.
fn render_clip_alpha(
    clip: &usvg::ClipPath,
    parent_transform: Transform,
    width: usize,
    height: usize,
    region: &std::rc::Rc<LayerRegion>,
    scale: f32,
    device_origin: Point64,
    svg_image_paths: &SvgImagePathSources,
) -> Option<ZeroedLayer> {
    // Clip geometry only intersects the clipped node's bounds (applied from
    // go_group_clip_rect); it is not one of its primitives.
    let recording = go_band_recording();
    if recording {
        GO_BAND_RECORD.with(|record| record.borrow_mut().push(Vec::new()));
    }
    let alpha = render_clip_alpha_unrecorded(
        clip,
        parent_transform,
        width,
        height,
        region,
        scale,
        device_origin,
        svg_image_paths,
    );
    if recording {
        GO_BAND_RECORD.with(|record| record.borrow_mut().pop());
    }
    alpha
}

fn render_clip_alpha_unrecorded(
    clip: &usvg::ClipPath,
    parent_transform: Transform,
    width: usize,
    height: usize,
    region: &std::rc::Rc<LayerRegion>,
    scale: f32,
    device_origin: Point64,
    svg_image_paths: &SvgImagePathSources,
) -> Option<ZeroedLayer> {
    let mut alpha = ZeroedLayer::new(region.len);
    if let Some(source) = svg_image_paths.native_image_clip(clip.id()) {
        let device_transform = parent_transform
            .post_concat(clip.transform())
            .post_scale(scale, scale);
        let mut transform = canonical_transform(device_transform);
        transform.tx -= device_origin.x;
        transform.ty -= device_origin.y;
        let paths = collect_raw_subpaths(source, flatten_tolerance64(transform), transform)?;
        let mut rasterizer = Rasterizer::new(width, height, 0.375);
        rasterizer.set_rounding_origin(Point64 {
            x: -device_origin.x,
            y: -device_origin.y,
        });
        for path in paths {
            let mapped = map_centerline(path, transform);
            rasterizer.add_fill_polyline(&mapped.0, mapped.1);
        }
        rasterizer.set_output_region(region.clone());
        let mut coverage = ZeroedLayer::new(alpha.len().checked_mul(4)?);
        rasterizer.paint(&mut coverage, [255; 4]);
        for (value, pixel) in alpha.iter_mut().zip(coverage.chunks_exact(4)) {
            *value = pixel[3];
        }
    } else {
        with_coverage_scratch(alpha.len().checked_mul(4)?, |coverage| {
            render_clip_node(
                clip.root(),
                parent_transform,
                &mut alpha,
                coverage,
                width,
                height,
                region,
                scale,
                device_origin,
            )
        })?;
    }
    if let Some(parent) = clip.clip_path() {
        let parent_alpha = render_clip_alpha(
            parent,
            parent_transform,
            width,
            height,
            region,
            scale,
            device_origin,
            svg_image_paths,
        )?;
        for (value, parent_value) in alpha.iter_mut().zip(parent_alpha.iter()) {
            *value = ((*value as u16 * *parent_value as u16 + 127) / 255) as u8;
        }
    }
    Some(alpha)
}

fn render_clip_node(
    group: &usvg::Group,
    parent_transform: Transform,
    alpha: &mut [u8],
    coverage: &mut [u8],
    width: usize,
    height: usize,
    region: &std::rc::Rc<LayerRegion>,
    scale: f32,
    device_origin: Point64,
) -> Option<()> {
    for child in group.children() {
        match child {
            usvg::Node::Group(group) => render_clip_node(
                group,
                parent_transform.post_concat(group.abs_transform()),
                alpha,
                coverage,
                width,
                height,
                region,
                scale,
                device_origin,
            )?,
            usvg::Node::Path(path) => {
                #[cfg(feature = "diagnostic-traces")]
                if raster_trace_pixel().is_some() {
                    eprintln!(
                        "d2-rs-raster clip-path transform={:?} bounds={:?} fill={:?}",
                        path.abs_transform(),
                        path.abs_bounding_box(),
                        path.fill(),
                    );
                }
                let Some(fill) = path.fill() else { continue };
                if fill.rule() != FillRule::NonZero {
                    return None;
                }
                let device_transform = parent_transform
                    .post_concat(path.abs_transform())
                    .post_scale(scale, scale);
                let mut rasterizer = Rasterizer::new(width, height, 0.375);
                let tolerance = flatten_tolerance(device_transform);
                let mut transform = canonical_transform(device_transform);
                transform.tx -= device_origin.x;
                transform.ty -= device_origin.y;
                rasterizer.set_rounding_origin(Point64 {
                    x: -device_origin.x,
                    y: -device_origin.y,
                });
                for (points, closed) in collect_subpaths(path.data(), tolerance, false) {
                    let mapped = map_centerline((points, closed), transform);
                    rasterizer.add_fill_polyline(&mapped.0, mapped.1);
                }
                // As in render_mask_node: zero coverage leaves max() alone,
                // so merge (and clear) only the fill's bounds in the region.
                let Some(bounds) = rasterizer.fill_pixel_bounds() else {
                    continue;
                };
                if !region.intersects(bounds) {
                    continue;
                }
                rasterizer.set_output_region(region.clone());
                let paint_alpha = (255.0 * fill.opacity().get()).clamp(0.0, 255.0) as u8;
                rasterizer.paint(coverage, [255, 255, 255, paint_alpha]);
                region.for_each_index_in(bounds, |index| {
                    let coverage_pixel = &mut coverage[index * 4..index * 4 + 4];
                    alpha[index] = alpha[index].max(coverage_pixel[3]);
                    coverage_pixel.fill(0);
                });
            }
            usvg::Node::Image(_) | usvg::Node::Text(_) => return None,
        }
    }
    Some(())
}

fn draw_group_contents(
    tree: &Tree,
    group: &usvg::Group,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    text_outline: bool,
    ignore_masks: bool,
    reference_origin: Point64,
    inside_masked_layer: bool,
    effect_path_reference_origin: Option<Point64>,
    raster_sources: &RasterSourceMap<'_>,
    svg_image_paths: &SvgImagePathSources,
) -> bool {
    let opacity = opacity * group.opacity().get();
    draw_group_children(
        tree,
        group,
        pixels,
        width,
        height,
        scale,
        opacity,
        text_outline,
        ignore_masks,
        reference_origin,
        inside_masked_layer,
        effect_path_reference_origin,
        raster_sources,
        svg_image_paths,
    )
}

fn draw_group_children(
    tree: &Tree,
    group: &usvg::Group,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    text_outline: bool,
    ignore_masks: bool,
    reference_origin: Point64,
    inside_masked_layer: bool,
    effect_path_reference_origin: Option<Point64>,
    raster_sources: &RasterSourceMap<'_>,
    svg_image_paths: &SvgImagePathSources,
) -> bool {
    for child in group.children() {
        let ok = match child {
            usvg::Node::Group(group) => draw_group_impl(
                tree,
                group,
                pixels,
                width,
                height,
                scale,
                opacity,
                text_outline,
                false,
                Some(reference_origin),
                inside_masked_layer,
                effect_path_reference_origin,
                raster_sources,
                svg_image_paths,
            ),
            usvg::Node::Path(path) if path.is_visible() => {
                #[cfg(feature = "diagnostic-traces")]
                if let Some((trace_x, trace_y)) = raster_trace_pixel() {
                    let bounds = path.abs_stroke_bounding_box();
                    let left = f64::from(bounds.left()) * f64::from(scale);
                    let top = f64::from(bounds.top()) * f64::from(scale);
                    let right = f64::from(bounds.right()) * f64::from(scale);
                    let bottom = f64::from(bounds.bottom()) * f64::from(scale);
                    if (trace_x as f64) >= left
                        && (trace_x as f64) < right
                        && (trace_y as f64) >= top
                        && (trace_y as f64) < bottom
                    {
                        eprintln!(
                            "d2-rs-raster path id={:?} parent_group={:?} bounds=({left:.3},{top:.3},{right:.3},{bottom:.3}) mask={} clip={} fill={:?} stroke={:?}",
                            path.id(),
                            group.id(),
                            group.mask().is_some(),
                            group.clip_path().is_some(),
                            path.fill(),
                            path.stroke(),
                        );
                    }
                }
                let nontrivial_mask = group
                    .mask()
                    .is_some_and(|mask| !is_opaque_white_mask(mask, group));
                let path_reference_origin = if inside_masked_layer || nontrivial_mask {
                    reference_origin
                } else {
                    effect_path_reference_origin.unwrap_or(Point64 { x: 0.0, y: 0.0 })
                };
                let mut path_rounding_origin = path_reference_origin;
                if inside_masked_layer {
                    path_rounding_origin.x += 2.0;
                }
                draw_path(
                    path,
                    pixels,
                    width,
                    height,
                    scale,
                    opacity,
                    text_outline,
                    path_reference_origin,
                    path_rounding_origin,
                    raster_sources,
                    svg_image_paths.raw_path(path.id()),
                )
            }
            usvg::Node::Path(_) => true,
            usvg::Node::Image(_) if raster_sources.banded => return false,
            // Image primitives (and a vector image's viewport clip) bound
            // their node by transformedImagePixelBounds; images are sampled,
            // not rasterized, so nothing below would record them.
            usvg::Node::Image(image) if go_band_recording() => {
                let bounds = image.abs_bounding_box();
                let scale = f64::from(scale);
                let leaf = GoLeafBounds::Rect(
                    (f64::from(bounds.left()) * scale).floor(),
                    (f64::from(bounds.top()) * scale).floor(),
                    (f64::from(bounds.right()) * scale).ceil(),
                    (f64::from(bounds.bottom()) * scale).ceil(),
                );
                GO_BAND_RECORD.with(|record| {
                    if let Some(record) = record.borrow_mut().last_mut() {
                        record.push((leaf, None));
                    }
                });
                true
            }
            // A band pass skips an image whose box, with a margin for the
            // native placement's precision and antialiasing, misses the
            // band: image painters write only inside their clipped box.
            usvg::Node::Image(image)
                if frame_band_rows(width, height).is_some_and(|(top, bottom)| {
                    let bounds = image.abs_bounding_box();
                    let margin = 8.0 * f64::from(scale) + 2.0;
                    let first = (f64::from(bounds.top()) * f64::from(scale) - margin).floor();
                    let end = (f64::from(bounds.bottom()) * f64::from(scale) + margin).ceil();
                    // Keep images no band reaches, so a failure still surfaces.
                    end > 0.0
                        && first < height as f64
                        && (end <= top as f64 || first >= bottom as f64)
                }) =>
            {
                true
            }
            usvg::Node::Image(image) => draw_svg_image(
                image,
                pixels,
                width,
                height,
                scale,
                opacity,
                text_outline,
                svg_image_paths.image_paths(group.id()),
                svg_image_paths.image_paths_by_id(group.id()),
                svg_image_paths.scene_image(group.id()),
                svg_image_paths.image_group_opacities(group.id()),
                svg_image_paths.native_image(group.id()),
            ),
            // usvg has already shaped text and exposes its positioned glyphs
            // as a flattened path group. Feeding those outlines through the
            // same analytic kernel as ordinary scene paths keeps glyph
            // coverage on the D2 raster boundary instead of switching back
            // to resvg's independent text antialiaser.
            usvg::Node::Text(text) => {
                #[cfg(feature = "diagnostic-traces")]
                if raster_trace_pixel().is_some() {
                    eprintln!(
                        "d2-rs-text-parent bounds={:?} opacity={:.9} blend={:?} isolate={} filters={} clip={} mask={} inside_masked_layer={inside_masked_layer} reference_origin=({:.3},{:.3})",
                        group.abs_bounding_box(),
                        group.opacity().get(),
                        group.blend_mode(),
                        group.isolate(),
                        group.filters().len(),
                        group.clip_path().is_some(),
                        group.mask().is_some(),
                        reference_origin.x,
                        reference_origin.y,
                    );
                }
                // A band pass skips a text whose glyphs, as drawn in an
                // earlier pass with the same placement, lie outside the band.
                let draw_key = (
                    text.as_ref() as *const usvg::Text as usize,
                    [
                        reference_origin.x.to_bits(),
                        reference_origin.y.to_bits(),
                        raster_sources.device_origin.x.to_bits(),
                        raster_sources.device_origin.y.to_bits(),
                    ],
                    (width, height, scale.to_bits(), opacity.to_bits()),
                );
                if !go_band_recording()
                    && let Some((top, bottom)) = frame_band_rows(width, height)
                    && let Some((first, end)) =
                        text_memo(|memo| &mut memo.text_rows, draw_key, || None)
                    && (end <= top as isize || first >= bottom as isize)
                {
                    continue;
                }
                let color_text = text_contains_color_glyph(tree, text);
                if color_text && raster_sources.banded {
                    return false;
                }
                let probe =
                    PAINT_ROW_PROBE.with(|probe| probe.replace(Some((isize::MAX, isize::MIN))));
                let drawn = if color_text {
                    Some(draw_color_text_with_local_transforms(
                        child,
                        text,
                        tree,
                        pixels,
                        width,
                        height,
                        scale,
                        opacity,
                        raster_sources
                            .native_text_transforms
                            .get(&(text.as_ref() as *const usvg::Text))
                            .copied(),
                    ))
                } else {
                    draw_text_fixed(
                        text,
                        tree,
                        pixels,
                        width,
                        height,
                        scale,
                        opacity,
                        reference_origin,
                        raster_sources
                            .paint_reference_contexts
                            .get(text.id())
                            .copied(),
                        raster_sources
                            .native_text_transforms
                            .get(&(text.as_ref() as *const usvg::Text))
                            .copied(),
                        raster_sources.device_origin,
                        raster_sources.banded,
                        svg_image_paths,
                    )
                };
                let painted_rows = PAINT_ROW_PROBE.with(|row_probe| {
                    let painted = row_probe.get();
                    // An enclosing probe still sees these rows.
                    row_probe.set(match (probe, painted) {
                        (Some(outer), Some(inner)) => {
                            Some((outer.0.min(inner.0), outer.1.max(inner.1)))
                        }
                        (outer, _) => outer,
                    });
                    painted
                });
                if drawn == Some(true)
                    && let Some(rows) = painted_rows
                {
                    text_memo_store(|memo| &mut memo.text_rows, draw_key, Some(rows));
                }
                drawn.unwrap_or_else(|| {
                    // A native code handoff contains only parser prototypes.
                    // Unsupported layouts must materialize the original code
                    // in the whole-render fallback, never paint those glyphs.
                    if svg_image_paths.code_line(text.id()).is_some() {
                        return false;
                    }
                    draw_group_impl(
                        tree,
                        text.flattened(),
                        pixels,
                        width,
                        height,
                        scale,
                        opacity,
                        true,
                        ignore_masks,
                        Some(reference_origin),
                        inside_masked_layer,
                        effect_path_reference_origin,
                        raster_sources,
                        svg_image_paths,
                    )
                })
            }
        };
        if !ok {
            return false;
        }
    }
    true
}

fn draw_effect_group(
    tree: &Tree,
    group: &usvg::Group,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    inherited_opacity: f32,
    group_opacity: f32,
    text_outline: bool,
    inherited_reference_origin: Option<Point64>,
    inside_masked_layer: bool,
    effect_path_reference_origin: Option<Point64>,
    raster_sources: &RasterSourceMap<'_>,
    svg_image_paths: &SvgImagePathSources,
) -> bool {
    let reference_origin = if group
        .children()
        .iter()
        .any(|child| matches!(child, usvg::Node::Text(_)))
    {
        group_reference_origin_with_text_transform(group, scale, true)
    } else {
        inherited_reference_origin
            .unwrap_or_else(|| group_reference_origin_with_text_transform(group, scale, true))
    };
    // Source setBandReferenceBounds assigns each effect's visible prepared
    // bounds to descendant path/stroke rasterizers. Opaque intermediate groups
    // inherit that origin; text and masks retain their separate rounding rules.
    let prepared_origin = group_reference_origin_with_text_transform(group, scale, true);
    let parent_origin = effect_path_reference_origin.unwrap_or(Point64 { x: 0.0, y: 0.0 });
    let path_origin = Point64 {
        x: prepared_origin.x.max(parent_origin.x),
        y: prepared_origin.y.max(parent_origin.y),
    };
    let mut layer = take_effect_layer(pixels.len());
    let scope = LayerPaintScope::begin(tree, group, &layer);
    if !draw_group_children(
        tree,
        group,
        &mut layer,
        width,
        height,
        scale,
        inherited_opacity,
        text_outline,
        false,
        reference_origin,
        inside_masked_layer,
        Some(path_origin),
        raster_sources,
        svg_image_paths,
    ) {
        return false;
    }
    let region = scope.finish(width, height);
    debug_assert_layer_in_region(&layer, width, &region);
    composite_effect_group(
        pixels,
        &layer,
        width,
        height,
        // Source composites prepared paint bounds, including stroke ink.
        // Geometry bounds alone clip off strokes at opacity-layer edges.
        effect_composite_bounds(group, scale, raster_sources.device_origin, width, height),
        &region,
        scale,
        svg_image_paths.exact_effect_opacity(group.id(), group_opacity),
        group.blend_mode(),
    );
    recycle_effect_layer(layer, width, height, region);
    true
}

// usvg 0.48's COLRv1 flattening composes a local font-to-text group but
// leaves descendant abs_transform values in font units. Source
// d2renderers/d2raster/colrv1.go applies fontToText and clip together.
// Keep analytic rendering for surrounding scene paths, and use Rust's local
// transform renderer only for color text until the exact paint kernel is ported.
fn text_contains_color_glyph(tree: &Tree, text: &usvg::Text) -> bool {
    text_memo(
        |memo| &mut memo.color,
        text as *const usvg::Text as usize,
        || text_contains_color_glyph_uncached(tree, text),
    )
}

fn text_contains_color_glyph_uncached(tree: &Tree, text: &usvg::Text) -> bool {
    let native = native_mixed_font_layout(tree, text);
    let spans = native
        .as_deref()
        .map_or(text.layouted(), |(spans, _)| spans.as_slice());
    spans.iter().any(|span| {
        span.positioned_glyphs.iter().any(|glyph| {
            // Go's preparedFont retains glyph/face metadata. Repeated labels
            // must not reparse the same font for each color-presence probe.
            text_memo(
                |memo| &mut memo.color_glyph_presence,
                (tree as *const Tree as usize, glyph.font, glyph.id.0),
                || {
                    tree.fontdb()
                        .with_face_data(glyph.font, |data, index| {
                            ttf_parser::Face::parse(data, index).is_ok_and(|face| {
                                u16::try_from(glyph.id.0)
                                    .is_ok_and(|id| face.is_color_glyph(ttf_parser::GlyphId(id)))
                            })
                        })
                        .unwrap_or(false)
                },
            )
        })
    })
}

// Source d2raster/text.go positions color and monochrome glyphs from the
// same grapheme-assigned advances. Reuse retained COLR paint trees only when
// every paintable glyph is present with the same face/ID/order; keep their
// internal paint geometry and replace only the glyph-local placement.
fn native_color_glyph_placements<'a>(
    tree: &Tree,
    text: &'a usvg::Text,
) -> Option<Vec<(&'a usvg::Node, Transform)>> {
    let layout = native_mixed_font_layout(tree, text)?;
    let (spans, positions) = layout.as_ref();
    let is_colr = |glyph: &usvg::layout::PositionedGlyph| {
        tree.fontdb()
            .with_face_data(glyph.font, |data, index| {
                ttf_parser::Face::parse(data, index)
                    .ok()
                    .is_some_and(|face| {
                        u16::try_from(glyph.id.0).is_ok_and(|id| {
                            face.tables().colr.is_some()
                                && face.is_color_glyph(ttf_parser::GlyphId(id))
                        })
                    })
            })
            .unwrap_or(false)
    };
    let old = text
        .layouted()
        .iter()
        .flat_map(|span| &span.positioned_glyphs)
        .filter(|glyph| is_colr(glyph))
        .collect::<Vec<_>>();
    let mut shaped = Vec::new();
    for (span, transforms) in spans.iter().zip(positions) {
        for (glyph, transform) in span.positioned_glyphs.iter().zip(transforms) {
            if is_colr(glyph) {
                shaped.push((glyph, *transform));
            } else if !glyph
                .text
                .chars()
                .all(|value| value.is_whitespace() || crate::font_shaping::ignorable(value))
            {
                // Monochrome paint and decorations need the shared native
                // paint pipeline before their flattened trees can be reused.
                return None;
            }
        }
    }
    let children = text.flattened().children();
    if old.is_empty() || old.len() != shaped.len() || children.len() != old.len() {
        return None;
    }
    let mut result = Vec::new();
    for ((old, (new, position)), child) in old.into_iter().zip(shaped).zip(children) {
        if old.font != new.font || old.id != new.id || !matches!(child, usvg::Node::Group(_)) {
            return None;
        }
        // Only authenticated bundled COLRv1 plans can reach this native
        // mapping. Full Source paint compilation supplies the next stage.
        tree.fontdb().with_face_data(new.font, |bytes, _| {
            crate::colr_plan::Compiler::new(bytes)
                .ok()?
                .compile(new.id.0)
                .ok()?
        })??;
        let correction = affine_mul(
            position,
            affine_inverse(glyph_transform64(old.colr_transform()))?,
        );
        let correction = Transform::from_row(
            correction.sx as f32,
            correction.ky as f32,
            correction.kx as f32,
            correction.sy as f32,
            correction.tx as f32,
            correction.ty as f32,
        );
        result.push((child, correction));
    }
    Some(result)
}

fn draw_color_text_native(
    tree: &Tree,
    text: &usvg::Text,
    destination: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    precise_device_transform: Option<Affine64>,
) -> Option<bool> {
    let layout = native_mixed_font_layout(tree, text)?;
    let (spans, positions) = layout.as_ref();
    let outer = precise_device_transform.unwrap_or_else(|| {
        affine_mul(
            Affine64 {
                sx: f64::from(scale),
                sy: f64::from(scale),
                kx: 0.0,
                ky: 0.0,
                tx: 0.0,
                ty: 0.0,
            },
            glyph_transform64(text.abs_transform()),
        )
    });
    // Source prepareText's vector root has opacity one and renders its
    // monochrome children directly into the destination. Paint there, but
    // keep the original of every row before it is first painted, so an
    // unsupported run restores the destination before falling back.
    let direct = opacity == 1.0;
    // A band pass paints only its own rows (band-limited rasterizers and
    // per-pixel compositing), so other rows are never painted.
    let (band_top, band_bottom) = frame_band_rows(width, height).unwrap_or((0, height));
    let row_bytes = width * 4;
    let mut layer = (!direct).then(|| ZeroedLayer::new(destination.len()));
    let mut undo = RowUndo::default();
    // Rows `first..end` are about to be painted.
    let touch = |destination: &[u8], first: usize, end: usize, undo: &mut RowUndo| {
        let (first, end) = (first.max(band_top), end.min(band_bottom));
        if first < end {
            undo.save(destination, row_bytes, first, end);
        }
    };
    let painted = (|| -> Option<()> {
        for (span, positions) in spans.iter().zip(positions) {
            if span.stroke.is_some() || span.overline.is_some() || span.line_through.is_some() {
                return None;
            }
            let foreground = solid_paint(
                span.fill.as_ref()?.paint(),
                span.fill.as_ref()?.opacity().get(),
            )?;
            let size = (f64::from(span.font_size.get()) * 64.0).round() / 64.0;
            let mut pending: Vec<Vec<Point64>> = Vec::new();
            let flush = |pending: &mut Vec<Vec<Point64>>,
                         destination: &mut [u8],
                         layer: &mut Option<ZeroedLayer>,
                         undo: &mut RowUndo| {
                if pending.is_empty() {
                    return;
                }
                // The rasterizer writes only rows its edges span.
                let (low, high) = pending
                    .iter()
                    .flatten()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                        (low.min(point.y), high.max(point.y))
                    });
                let first = (low.floor() - 1.0).clamp(0.0, height as f64) as usize;
                let end = (high.ceil() + 2.0).clamp(0.0, height as f64) as usize;
                touch(destination, first, end, undo);
                let target: &mut [u8] = match layer {
                    Some(layer) => layer,
                    None => destination,
                };
                colr_paint::plain(pending, foreground, target, width, height);
                pending.clear();
            };
            for (glyph, position) in span.positioned_glyphs.iter().zip(positions) {
                tree.fontdb()
                    .with_face_data(glyph.font, |data, index| -> Option<()> {
                        let face = ttf_parser::Face::parse(data, index).ok()?;
                        let units = f64::from(face.units_per_em());
                        let glyph_base = Affine64 {
                            sx: size / units,
                            sy: -size / units,
                            kx: 0.0,
                            ky: 0.0,
                            tx: position.tx,
                            ty: position.ty,
                        };
                        let base = affine_mul(outer, glyph_base);
                        let glyph_id = u16::try_from(glyph.id.0).ok()?;
                        if face.is_color_glyph(ttf_parser::GlyphId(glyph_id)) {
                            flush(&mut pending, destination, &mut layer, &mut undo);
                            let plan = text_memo(
                                |memo| &mut memo.color_plans,
                                (data.as_ptr() as usize, glyph.id.0),
                                || {
                                    crate::colr_plan::Compiler::new(data)
                                        .ok()?
                                        .compile(glyph.id.0)
                                        .ok()?
                                        .map(std::rc::Rc::new)
                                },
                            )?;
                            let prepared = text_memo(
                                |memo| &mut memo.color_glyphs,
                                (
                                    data.as_ptr() as usize,
                                    glyph.id.0,
                                    [base.sx, base.ky, base.kx, base.sy, base.tx, base.ty]
                                        .map(f64::to_bits),
                                    width,
                                    height,
                                ),
                                || {
                                    colr_paint::prepare_glyph(&plan, &face, base, width, height)
                                        .map(std::rc::Rc::new)
                                },
                            )?;
                            let pixels = colr_paint::paint_glyph(
                                &prepared,
                                width,
                                height,
                                band_top..band_bottom,
                            );
                            let rows = pixels.rows();
                            touch(destination, rows.start, rows.end, &mut undo);
                            let target: &mut [u8] = match &mut layer {
                                Some(layer) => layer,
                                None => destination,
                            };
                            pixels.merge_into(target, width);
                            // A subsequent glyph in the same cluster can have empty
                            // retained text and still carry a real combining-mark
                            // outline.
                        } else if glyph.id.0 != 0
                            && (glyph.text.is_empty()
                                || !glyph.text.chars().all(|value| {
                                    value.is_whitespace() || crate::font_shaping::ignorable(value)
                                }))
                        {
                            // Source sfnt.LoadGlyph supports both glyf and CFF
                            // outlines. Reuse the fixed outline boundary so a CFF
                            // fallback does not send the complete mixed
                            // monochrome/color run to resvg.
                            let has_glyf = face
                                .raw_face()
                                .table(ttf_parser::Tag::from_bytes(b"glyf"))
                                .is_some();
                            // An empty glyf record (e.g. the ZWJ inside an emoji
                            // sequence) is a source Empty glyph: it draws nothing.
                            if has_glyf
                                && raw_glyf_record(&face, glyph_id).is_some_and(<[u8]>::is_empty)
                            {
                                return Some(());
                            }
                            let path = if has_glyf {
                                let raw = raw_go_glyph(&face, glyph_id, 0)?;
                                fixed_path_from_raw(&raw, glyph_base, size / units, -size / units)
                            } else {
                                fixed_glyph_path(tree, glyph, glyph_base)?
                            };
                            for (points, _) in
                                collect_fixed_subpaths(&path, 0.25 / affine_max_scale(outer))
                            {
                                pending.push(
                                    points.into_iter().map(|p| map_point(p, outer)).collect(),
                                );
                            }
                        }
                        Some(())
                    })??;
            }
            flush(&mut pending, destination, &mut layer, &mut undo);
            // d2raster/text.go appends underline geometry after the color
            // and monochrome parts, using the primary font's metrics.
            if span.underline.is_some() {
                let mut rasterizer = Rasterizer::new(width, height, 0.375);
                if add_native_text_underline(
                    tree,
                    text,
                    span,
                    positions,
                    outer,
                    &mut rasterizer,
                    f64::from(span.font_size.get()),
                )? {
                    if let Some((_, first, _, end)) = rasterizer.fill_pixel_bounds() {
                        touch(destination, first, end, &mut undo);
                    }
                    let target: &mut [u8] = match &mut layer {
                        Some(layer) => layer,
                        None => destination,
                    };
                    rasterizer.paint(target, foreground);
                }
            }
        }
        Some(())
    })();
    if painted.is_none() {
        undo.restore(destination, row_bytes);
        return None;
    }
    let Some(layer) = layer else {
        return Some(true);
    };
    let alpha = (f64::from(opacity) * 255.0).round().clamp(0.0, 255.0) as u8;
    // The layer is transparent outside the rows it received.
    let rows = undo.first * row_bytes..undo.end * row_bytes;
    for (d, s) in destination[rows.clone()]
        .chunks_exact_mut(4)
        .zip(layer[rows].chunks_exact(4))
    {
        let mut s: [u8; 4] = s.try_into().unwrap();
        scale_premultiplied(&mut s, alpha);
        blend_premultiplied_pixel(d, s);
    }
    Some(true)
}

/// The original bytes of a contiguous range of destination rows, saved
/// before each row is first painted (draw_color_text_native).
#[derive(Default)]
struct RowUndo {
    first: usize,
    end: usize,
    saved: Vec<u8>,
}

impl RowUndo {
    fn save(&mut self, destination: &[u8], row_bytes: usize, first: usize, end: usize) {
        if self.first == self.end {
            self.first = first;
            self.end = end;
            self.saved = destination[first * row_bytes..end * row_bytes].to_vec();
            return;
        }
        if first < self.first {
            let mut saved = destination[first * row_bytes..self.first * row_bytes].to_vec();
            saved.extend_from_slice(&self.saved);
            self.saved = saved;
            self.first = first;
        }
        if end > self.end {
            self.saved
                .extend_from_slice(&destination[self.end * row_bytes..end * row_bytes]);
            self.end = end;
        }
    }

    fn restore(&self, destination: &mut [u8], row_bytes: usize) {
        destination[self.first * row_bytes..self.end * row_bytes].copy_from_slice(&self.saved);
    }
}

fn draw_color_text_with_local_transforms(
    node: &usvg::Node,
    text: &usvg::Text,
    tree: &Tree,
    destination: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    precise_device_transform: Option<Affine64>,
) -> bool {
    if let Some(rendered) = draw_color_text_native(
        tree,
        text,
        destination,
        width,
        height,
        scale,
        opacity,
        precise_device_transform,
    ) {
        return rendered;
    }
    let Some(bounds) = node.abs_layer_bounding_box() else {
        return true;
    };
    // resvg evaluates its glyph and clip transforms in global f32 space.
    // Rebasing them to a smaller surface changes its gradient/clip results.
    // Keep those coordinates, but commit pages only for the glyph's ink.
    // The native COLR path above uses compact destinations without rebasing.
    PAINT_ROW_PROBE.with(|probe| {
        if let Some((first, end)) = probe.get() {
            probe.set(Some((first.min(0), end.max(height as isize))));
        }
    });
    let mut storage = ZeroedLayer::new(destination.len());
    let Some(mut layer) =
        resvg::tiny_skia::PixmapMut::from_bytes(&mut storage, width as u32, height as u32)
    else {
        return false;
    };
    // render_node removes the absolute layer origin. Restore it before
    // composing the text's inherited transform; glyph-local transforms and
    // clip masks are then evaluated recursively by resvg.
    let transform =
        resvg::tiny_skia::Transform::from_scale(scale, scale).pre_concat(text.abs_transform());
    if let Some(glyphs) = native_color_glyph_placements(tree, text) {
        for (glyph, correction) in glyphs {
            let Some(glyph_bounds) = glyph.abs_layer_bounding_box() else {
                return false;
            };
            let transform = transform
                .pre_concat(correction)
                .pre_translate(glyph_bounds.x(), glyph_bounds.y());
            if resvg::render_node(glyph, transform, &mut layer).is_none() {
                return false;
            }
        }
    } else if resvg::render_node(
        node,
        transform.pre_translate(bounds.x(), bounds.y()),
        &mut layer,
    )
    .is_none()
    {
        return false;
    }
    let opacity_byte = (f64::from(opacity) * 255.0).round().clamp(0.0, 255.0) as u8;
    for (destination, source) in destination
        .chunks_exact_mut(4)
        .zip(layer.data_mut().chunks_exact(4))
    {
        if source[3] != 0 {
            let mut source: [u8; 4] = source.try_into().unwrap();
            scale_premultiplied(&mut source, opacity_byte);
            blend_premultiplied_pixel(destination, source);
        }
    }
    true
}

/// Under Go band emulation every layer pixel lies in a source destination
/// (compositeLayer covers the whole tightened layer), so composite the full
/// surface; transparent layer pixels are skipped.
fn effect_composite_bounds(
    group: &usvg::Group,
    scale: f32,
    origin: Point64,
    width: usize,
    height: usize,
) -> resvg::tiny_skia::Rect {
    if go_band_windows(width, height).is_some() {
        // Layer pixels come only from Go-windowed paints and from effect
        // side buffers (shadows) whose source layer bounds include them.
        let scale = f64::from(scale);
        return resvg::tiny_skia::Rect::from_xywh(
            0.0,
            0.0,
            (width as f64 / scale) as f32,
            (height as f64 / scale) as f32,
        )
        .expect("finite surface bounds");
    }
    effect_bounds_in_surface(group, scale, origin)
}

fn effect_bounds_in_surface(
    group: &usvg::Group,
    scale: f32,
    origin: Point64,
) -> resvg::tiny_skia::Rect {
    let bounds = group.abs_layer_bounding_box().to_rect();
    resvg::tiny_skia::Rect::from_xywh(
        bounds.x() - (origin.x / f64::from(scale)) as f32,
        bounds.y() - (origin.y / f64::from(scale)) as f32,
        bounds.width(),
        bounds.height(),
    )
    .expect("translated finite prepared effect bounds")
}

fn composite_effect_group(
    destination: &mut [u8],
    source: &[u8],
    width: usize,
    height: usize,
    bounds: resvg::tiny_skia::Rect,
    // Where `source` can be nonzero (see `LayerPaintScope`). Transparent
    // source pixels composite nothing, so the scan stops at its edges.
    region: &LayerRegion,
    scale: f32,
    opacity: f64,
    blend_mode: usvg::BlendMode,
) {
    let opacity_byte = (opacity * 255.0).round().clamp(0.0, 255.0) as u8;
    if opacity_byte == 0 || go_band_recording() {
        return;
    }
    let opacity_scale = f64::from(opacity_byte) / 255.0;
    // effects.go compositeLayer composites the whole prepared layer. Go's
    // prepared bounds contain every painted primitive, including glyph ink
    // that usvg's layer bounding box omits when native text layout resolves
    // wider outlines, so composite every painted pixel of the layer.
    let _ = (bounds, scale);
    let (x_start, y_start, x_end, y_end) = (0, 0, width, height);
    for (row, row_x0, row_x1, _) in region.rows_in(y_start.min(height), y_end) {
        let (x_start, x_end) = (x_start.max(row_x0), x_end.min(row_x1));
        let row_start = (row * width + x_start.min(width)) * 4;
        for pixel_start in (row_start..(row * width + x_end) * 4).step_by(4) {
            let source_alpha_byte = source[pixel_start + 3];
            if source_alpha_byte == 0 {
                continue;
            }
            if blend_mode == usvg::BlendMode::Normal {
                let mut scaled_source = [
                    source[pixel_start],
                    source[pixel_start + 1],
                    source[pixel_start + 2],
                    source_alpha_byte,
                ];
                scale_premultiplied(&mut scaled_source, opacity_byte);
                blend_premultiplied_pixel(
                    &mut destination[pixel_start..pixel_start + 4],
                    scaled_source,
                );
                continue;
            }
            let source_alpha = f64::from(source_alpha_byte) / 255.0 * opacity_scale;
            if source_alpha == 0.0 {
                continue;
            }
            let destination_alpha_byte = destination[pixel_start + 3];
            let backdrop_alpha = f64::from(destination_alpha_byte) / 255.0;
            let output_alpha = source_alpha + backdrop_alpha * (1.0 - source_alpha);
            let output_alpha_byte = (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            for channel in 0..3 {
                let source_premultiplied =
                    f64::from(source[pixel_start + channel]) / 255.0 * opacity_scale;
                let backdrop_premultiplied = f64::from(destination[pixel_start + channel]) / 255.0;
                let source_color =
                    f64::from(source[pixel_start + channel]) / f64::from(source_alpha_byte);
                let backdrop_color = if destination_alpha_byte == 0 {
                    0.0
                } else {
                    f64::from(destination[pixel_start + channel])
                        / f64::from(destination_alpha_byte)
                };
                let mixed = match blend_mode {
                    usvg::BlendMode::Multiply => backdrop_color * source_color,
                    usvg::BlendMode::Darken => backdrop_color.min(source_color),
                    usvg::BlendMode::ColorBurn => {
                        if backdrop_color >= 1.0 {
                            1.0
                        } else if source_color <= 0.0 {
                            0.0
                        } else {
                            1.0 - ((1.0 - backdrop_color) / source_color).min(1.0)
                        }
                    }
                    usvg::BlendMode::Overlay => {
                        if backdrop_color <= 0.5 {
                            2.0 * backdrop_color * source_color
                        } else {
                            1.0 - 2.0 * (1.0 - backdrop_color) * (1.0 - source_color)
                        }
                    }
                    usvg::BlendMode::Lighten => backdrop_color.max(source_color),
                    _ => unreachable!("draw_group validates native blend modes"),
                };
                let output_premultiplied = source_premultiplied * (1.0 - backdrop_alpha)
                    + source_alpha * backdrop_alpha * mixed
                    + backdrop_premultiplied * (1.0 - source_alpha);
                destination[pixel_start + channel] = (output_premultiplied * 255.0)
                    .round()
                    .clamp(0.0, f64::from(output_alpha_byte))
                    as u8;
            }
            destination[pixel_start + 3] = output_alpha_byte;
        }
    }
    note_painted(destination, region);
}

/// Go's retained renderer rounds 64-bit glyph coordinates relative to the
/// owning prepared-node bounds before adding the cropped band's origin. The
/// simple Rust path paints the complete canvas, so recover that stable node
/// reference from the parent's actual painted children. Expanding by one
/// device pixel matches the inclusive pixel bounds used by D2's bands.
/// The absolute top-left of a dashed stroke's painted runs, or None when the
/// stroke is solid or its runs cannot be derived.
fn dashed_stroke_run_min(path: &usvg::Path, stroke: &usvg::Stroke) -> Option<(f64, f64)> {
    let dashes = stroke.dasharray()?;
    let transform = canonical_transform(path.abs_transform());
    let (points, closed) =
        collect_centerline(path.data(), flatten_tolerance(path.abs_transform()))?;
    let runs = Rasterizer::dashed_centerline_exact(
        &clean_stroke_points(&points, closed),
        closed,
        dashes,
        stroke.dashoffset(),
        exact_dash_parameters(path),
    );
    let (mut left, mut top) = (f64::INFINITY, f64::INFINITY);
    for point in runs.iter().flat_map(|run| &run.points) {
        let point = map_point(*point, transform);
        left = left.min(point.x);
        top = top.min(point.y);
    }
    (left.is_finite() && top.is_finite()).then_some((left, top))
}

fn group_reference_origin(group: &usvg::Group, scale: f32) -> Point64 {
    group_reference_origin_with_text_transform(group, scale, false)
}

fn group_reference_origin_with_text_transform(
    group: &usvg::Group,
    scale: f32,
    canvas_text: bool,
) -> Point64 {
    text_memo(
        |memo| &mut memo.group_origins,
        (
            group as *const usvg::Group as usize,
            scale.to_bits(),
            canvas_text,
        ),
        || group_reference_origin_uncached(group, scale, canvas_text),
    )
}

fn group_reference_origin_uncached(group: &usvg::Group, scale: f32, canvas_text: bool) -> Point64 {
    fn visit(
        node: &usvg::Node,
        min_x: &mut f64,
        min_text_y: &mut f64,
        min_paint_y: &mut f64,
        canvas_text: bool,
    ) {
        match node {
            usvg::Node::Group(group) => {
                for child in group.children() {
                    visit(child, min_x, min_text_y, min_paint_y, canvas_text);
                }
            }
            usvg::Node::Path(path) if path.is_visible() => {
                let bounds = path.abs_bounding_box();
                *min_x = (*min_x).min(f64::from(bounds.left()));
                // Source effect reference bounds include filled primitives,
                // not only text ink and stroked paths. Ignoring an earlier
                // background chooses an origin inside the painted surface
                // and truncates its upper rows in the scanline region.
                if path.fill().is_some() {
                    *min_paint_y = (*min_paint_y).min(f64::from(bounds.top()));
                }
                if let Some(stroke) = path.stroke() {
                    let transform = path.abs_transform();
                    let a = f64::from(transform.sx);
                    let b = f64::from(transform.kx);
                    let c = f64::from(transform.ky);
                    let d = f64::from(transform.sy);
                    let trace = a * a + b * b + c * c + d * d;
                    let determinant = a * d - b * c;
                    let discriminant = (trace * trace - 4.0 * determinant * determinant).max(0.0);
                    let max_scale = ((trace + discriminant.sqrt()) * 0.5).sqrt();
                    // Go's prepared primitive bounds use the conservative
                    // stroke-run extent, including the join and cap factor,
                    // plus one antialiasing pixel. Those bounds become the
                    // float32 reference origin for every descendant of a
                    // clipped connection geometry group. Using the raw path
                    // left edge here picks an origin several pixels too far
                    // right for mitered crowfoot branches, even though the
                    // painted geometry itself is identical.
                    let mut factor = if stroke.linecap() == usvg::LineCap::Square {
                        std::f64::consts::SQRT_2
                    } else {
                        1.0
                    };
                    if matches!(
                        stroke.linejoin(),
                        usvg::LineJoin::Miter | usvg::LineJoin::MiterClip
                    ) {
                        factor = factor.max(f64::from(stroke.miterlimit().get()));
                    }
                    let expansion = f64::from(stroke.width().get()) * max_scale * 0.5 * factor;
                    // geometry.go strokeRunPixelBounds bounds the painted dash
                    // runs, not the whole centerline: a dash pattern (or an
                    // animated offset) that opens with a gap moves the
                    // prepared node bounds, and so the effect-layer origin.
                    let (left, top) = dashed_stroke_run_min(path, stroke)
                        .unwrap_or((f64::from(bounds.left()), f64::from(bounds.top())));
                    *min_x = (*min_x).min(left - expansion);
                    *min_paint_y = (*min_paint_y).min(top - expansion);
                }
            }
            usvg::Node::Image(image) => {
                let bounds = image.abs_bounding_box();
                *min_x = (*min_x).min(f64::from(bounds.left()));
                *min_paint_y = (*min_paint_y).min(f64::from(bounds.top()));
            }
            usvg::Node::Text(text) => {
                // usvg's flattened Group aggregates local and absolute path
                // bounds separately. Map local ink once for canvas ancestry;
                // absolute ink already carries the owning Text transform.
                // bounding_box is text-local; abs_bounding_box already includes
                // the owning Text transform on flattened monochrome paths.
                let local = text.flattened().bounding_box();
                let bounds = if canvas_text {
                    local.transform(text.abs_transform())
                } else {
                    Some(text.flattened().abs_bounding_box())
                };
                if let Some(bounds) = bounds {
                    *min_x = (*min_x).min(f64::from(bounds.left()));
                    *min_text_y = (*min_text_y).min(f64::from(bounds.top()));
                }
            }
            usvg::Node::Path(_) => {}
        }
    }

    let bounds = group.abs_bounding_box();
    #[cfg(feature = "diagnostic-traces")]
    let direct_text = group
        .children()
        .iter()
        .any(|child| matches!(child, usvg::Node::Text(_)));
    // Source retains actual prepared primitive bounds. Ancestor groups can
    // contain font metric boxes or empty geometry groups at (0,0); neither
    // may choose the rounding origin of visible descendant glyphs/paths.
    let mut min_x = f64::INFINITY;
    let mut min_text_y = f64::INFINITY;
    let mut min_paint_y = f64::INFINITY;
    for child in group.children() {
        visit(
            child,
            &mut min_x,
            &mut min_text_y,
            &mut min_paint_y,
            canvas_text,
        );
    }
    let mut min_y = if min_text_y.is_finite() {
        min_text_y
    } else {
        f64::from(bounds.top())
    };
    if min_paint_y.is_finite() {
        min_y = min_y.min(min_paint_y);
    }
    if !min_x.is_finite() {
        min_x = f64::from(bounds.left());
    }
    let origin = Point64 {
        x: (min_x * f64::from(scale)).floor() - 1.0,
        y: (min_y * f64::from(scale)).floor() - 1.0,
    };
    #[cfg(feature = "diagnostic-traces")]
    if let Some((trace_x, trace_y)) = raster_trace_pixel() {
        let left = f64::from(bounds.left()) * f64::from(scale);
        let top = f64::from(bounds.top()) * f64::from(scale);
        let right = f64::from(bounds.right()) * f64::from(scale);
        let bottom = f64::from(bounds.bottom()) * f64::from(scale);
        if (trace_x as f64) >= left
            && (trace_x as f64) < right
            && (trace_y as f64) >= top
            && (trace_y as f64) < bottom
        {
            eprintln!(
                "d2-rs-reference-group direct_text={direct_text} origin=({:.3},{:.3}) bounds=({:.3},{:.3},{:.3},{:.3}) stroke_bounds={:?} layer_bounds={:?} min_x={min_x:.6} min_text_y={min_text_y:.6} min_paint_y={min_paint_y:.6}",
                origin.x,
                origin.y,
                left,
                top,
                right,
                bottom,
                group.abs_stroke_bounding_box(),
                group.abs_layer_bounding_box(),
            );
        }
    }
    origin
}

fn draw_svg_image(
    image: &usvg::Image,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    text_outline: bool,
    source_paths: Option<&[SvgPath]>,
    source_paths_by_id: Option<&HashMap<String, SvgPath>>,
    scene_image: Option<&crate::svg_precision::SceneImagePlacement>,
    group_opacities: Option<&HashMap<String, f64>>,
    native: Option<crate::svg_precision::NativeImage<'_>>,
) -> bool {
    if let Some(native) = native {
        if let Some(result) = owned_png_scene::draw(
            image,
            native,
            scene_image,
            pixels,
            width,
            height,
            scale,
            opacity,
        ) {
            return result;
        }
    }
    let raster_sources = RasterSourceMap::default();
    if let usvg::ImageKind::PNG(data) = image.kind() {
        return crate::png_image::draw(
            image,
            data,
            pixels,
            width,
            height,
            scale,
            opacity,
            scene_image,
        );
    }
    let usvg::ImageKind::SVG(nested) = image.kind() else {
        return false;
    };
    let transform = image.abs_transform();
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("D2_RS_TRACE_SVG_RAW_PATHS").is_some() {
        eprintln!(
            "d2-rs-svg-import translate=({},{}) supported={} clip={} mask={} filters={} isolate={} raw={} ids={}",
            transform.tx,
            transform.ty,
            direct_svg_group_supported(nested.root(), source_paths, source_paths_by_id),
            nested.root().clip_path().is_some(),
            nested.root().mask().is_some(),
            nested.root().filters().len(),
            nested.root().isolate(),
            source_paths.map_or(0, |p| p.len()),
            source_paths_by_id.map_or(0, |p| p.len())
        );
    }
    #[cfg(feature = "diagnostic-traces")]
    if let Some((trace_x, trace_y)) = raster_trace_pixel() {
        let bounds = image.abs_bounding_box();
        let left = (f64::from(bounds.left()) * f64::from(scale)).floor() as isize;
        let top = (f64::from(bounds.top()) * f64::from(scale)).floor() as isize;
        let right = (f64::from(bounds.right()) * f64::from(scale)).ceil() as isize;
        let bottom = (f64::from(bounds.bottom()) * f64::from(scale)).ceil() as isize;
        if trace_x as isize >= left
            && (trace_x as isize) < right
            && trace_y as isize >= top
            && (trace_y as isize) < bottom
        {
            let root_bounds = nested.root().abs_bounding_box();
            let root_stroke_bounds = nested.root().abs_stroke_bounding_box();
            eprintln!(
                "d2-rs-svg-image bounds=({:.9},{:.9},{:.9},{:.9}) root=({:.9},{:.9},{:.9},{:.9}) stroke=({:.9},{:.9},{:.9},{:.9}) size=({:.9},{:.9}) transform=({:.9},{:.9},{:.9},{:.9},{:.9},{:.9}) scale={scale}",
                bounds.left(),
                bounds.top(),
                bounds.right(),
                bounds.bottom(),
                root_bounds.left(),
                root_bounds.top(),
                root_bounds.right(),
                root_bounds.bottom(),
                root_stroke_bounds.left(),
                root_stroke_bounds.top(),
                root_stroke_bounds.right(),
                root_stroke_bounds.bottom(),
                nested.size().width(),
                nested.size().height(),
                transform.sx,
                transform.kx,
                transform.ky,
                transform.sy,
                transform.tx,
                transform.ty,
            );
        }
    }
    // D2 renders a vector image's paths into a transparent, ink-bounds-sized
    // layer, clips that layer to the image viewport, then composites it over
    // the parent canvas. Keep that boundary for simple path-only assets too:
    // drawing directly onto an opaque node fill changes antialiased edges.
    // More complex assets continue through the nested renderer below.
    if opacity == 1.0
        && direct_svg_group_supported(nested.root(), source_paths, source_paths_by_id)
        && (direct_svg_group_fits_viewport(nested)
            || svg_viewport_has_integer_device_edges(image, scale)
            || source_paths.is_some_and(|paths| {
                paths.len() == svg_group_path_count(nested.root())
                    && paths.iter().all(|path| {
                        path.scene_transform.is_some()
                            && path.intrinsic_viewport.is_some()
                            && path.intrinsic_transforms.is_some()
                    })
            })
            || source_paths_by_id.is_some_and(|paths| {
                fn qualified(group: &usvg::Group, paths: &HashMap<String, SvgPath>) -> bool {
                    if retained_png_wrapper(group, Some(paths)).is_some() {
                        return true;
                    }
                    group.children().iter().all(|node| match node {
                        usvg::Node::Group(child) => qualified(child, paths),
                        usvg::Node::Path(path)
                            if !path.is_visible()
                                || (path.fill().is_none() && path.stroke().is_none()) =>
                        {
                            true
                        }
                        usvg::Node::Path(path) => paths.get(path.id()).is_some_and(|source| {
                            source.scene_transform.is_some()
                                && source.intrinsic_transforms.is_some()
                                && source.image_viewport.is_some()
                        }),
                        usvg::Node::Image(image) => {
                            !image.is_visible()
                                || retained_raster_image(group, image, Some(paths)).is_some_and(
                                    |source| {
                                        source.raster_image.is_some()
                                            && source.scene_transform.is_some()
                                            && source.intrinsic_transforms.is_some()
                                            && source.image_viewport.is_some()
                                    },
                                )
                        }
                        _ => false,
                    })
                }
                qualified(nested.root(), paths)
            }))
    {
        let Some((left, top, right, bottom)) = direct_svg_group_layer_bounds(
            image,
            nested,
            transform,
            scale,
            width,
            height,
            source_paths_by_id,
        ) else {
            return true;
        };
        let layer_width = right - left;
        let layer_height = bottom - top;
        let Some(layer_len) = layer_width
            .checked_mul(layer_height)
            .and_then(|value| value.checked_mul(4))
        else {
            return false;
        };
        let mut layer = vec![0_u8; layer_len];
        let raw_paths =
            source_paths.filter(|paths| paths.len() == svg_group_path_count(nested.root()));
        #[cfg(feature = "diagnostic-traces")]
        if std::env::var_os("D2_RS_TRACE_SVG_RAW_PATHS").is_some() {
            eprintln!(
                "d2-rs-svg-raw tagged_paths={} usvg_paths={} direct=true",
                raw_paths.map_or(0, <[SvgPath]>::len),
                svg_group_path_count(nested.root()),
            );
        }
        if !draw_svg_group_direct(
            nested.root(),
            &mut layer,
            layer_width,
            layer_height,
            scale,
            text_outline,
            transform,
            Point64 {
                x: left as f64,
                y: top as f64,
            },
            &raster_sources,
            raw_paths,
            source_paths_by_id,
            group_opacities,
        ) {
            return false;
        }
        if let Some(source) = source_paths_by_id.and_then(|paths| {
            paths
                .values()
                .find(|source| source.image_viewport.is_some())
        }) {
            if let (Some(scene), Some((display, size)), Some(viewport)) = (
                source.scene_transform,
                source.image_viewport,
                source.intrinsic_viewport,
            ) {
                let scale = f64::from(scale);
                let scene = Affine64 {
                    sx: scene[0] * scale,
                    ky: scene[1] * scale,
                    kx: scene[2] * scale,
                    sy: scene[3] * scale,
                    tx: scene[4] * scale,
                    ty: scene[5] * scale,
                };
                let intrinsic = source_affine_mul(
                    scene,
                    Affine64 {
                        sx: display[0],
                        ky: display[1],
                        kx: display[2],
                        sy: display[3],
                        tx: display[4],
                        ty: display[5],
                    },
                );
                for (mut transform, size) in [(intrinsic, size), (scene, viewport)] {
                    transform.tx -= left as f64;
                    transform.ty -= top as f64;
                    let points = vec![
                        Point64 { x: 0.0, y: 0.0 },
                        Point64 { x: size[0], y: 0.0 },
                        Point64 {
                            x: size[0],
                            y: size[1],
                        },
                        Point64 { x: 0.0, y: size[1] },
                    ];
                    let mapped = map_centerline((points, true), transform);
                    let mut clip = Rasterizer::new(layer_width, layer_height, 0.375);
                    clip.add_fill_polyline(&mapped.0, true);
                    let mut mask = vec![0_u8; layer.len()];
                    clip.paint(&mut mask, [255; 4]);
                    for (pixel, coverage) in layer.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                        scale_premultiplied(pixel, coverage[3]);
                    }
                }
            }
        }
        if let Some((matrix, viewport)) = raw_paths
            .and_then(|paths| paths.first())
            .and_then(|path| path.scene_transform.zip(path.intrinsic_viewport))
        {
            // d2scenebuild.resolveLatexAsset always retains a viewport Clip.
            // Even fitting ink must multiply partial edge-pixel coverage.
            let scale = f64::from(scale);
            let transform = Affine64 {
                sx: matrix[0] * scale,
                ky: matrix[1] * scale,
                kx: matrix[2] * scale,
                sy: matrix[3] * scale,
                tx: matrix[4] * scale - left as f64,
                ty: matrix[5] * scale - top as f64,
            };
            let points = [
                Point64 { x: 0.0, y: 0.0 },
                Point64 {
                    x: viewport[0],
                    y: 0.0,
                },
                Point64 {
                    x: viewport[0],
                    y: viewport[1],
                },
                Point64 {
                    x: 0.0,
                    y: viewport[1],
                },
            ];
            let mapped = map_centerline((points.to_vec(), true), transform);
            let mut clip = Rasterizer::new(layer_width, layer_height, 0.375);
            clip.add_fill_polyline(&mapped.0, true);
            let mut mask = vec![0_u8; layer.len()];
            clip.paint(&mut mask, [255; 4]);
            // Source retains both the vector asset Clip and the Image box Clip.
            for (pixel, mask) in layer.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                scale_premultiplied(pixel, mask[3]);
                scale_premultiplied(pixel, mask[3]);
            }
        }
        for local_y in 0..layer_height {
            let target_y = top + local_y;
            for local_x in 0..layer_width {
                let source_index = (local_y * layer_width + local_x) * 4;
                let alpha = layer[source_index + 3];
                if alpha == 0 {
                    continue;
                }
                let target_index = ((target_y * width + left + local_x) * 4) as usize;
                blend_premultiplied_pixel(
                    &mut pixels[target_index..target_index + 4],
                    [
                        layer[source_index],
                        layer[source_index + 1],
                        layer[source_index + 2],
                        alpha,
                    ],
                );
            }
        }
        return true;
    }
    // Source assets.go clips imported content to its intrinsic viewport,
    // then image.go composes the display mapping and the outer viewport.
    // Rendering an intrinsic-size bitmap and copying it at an integer origin
    // loses that mapping (and rotation/shear) before the first paint.
    // Keep the Rust subtree renderer on the composed device transform until
    // all imported effect/text paths are qualified for the analytic kernel.
    let Some(mut layer) = resvg::tiny_skia::Pixmap::new(width as u32, height as u32) else {
        return false;
    };
    let device_transform = transform.post_scale(scale, scale);
    resvg::render(nested, device_transform, &mut layer.as_mut());
    let mut clip = vec![0_u8; pixels.len()];
    let mut rasterizer = Rasterizer::new(width, height, 0.375);
    let intrinsic = image.size();
    let viewport = [
        Point64 { x: 0.0, y: 0.0 },
        Point64 {
            x: f64::from(intrinsic.width()),
            y: 0.0,
        },
        Point64 {
            x: f64::from(intrinsic.width()),
            y: f64::from(intrinsic.height()),
        },
        Point64 {
            x: 0.0,
            y: f64::from(intrinsic.height()),
        },
    ];
    let viewport = viewport.map(|point| map_point(point, canonical_transform(device_transform)));
    rasterizer.add_fill_polyline(&viewport, true);
    rasterizer.paint(&mut clip, [255; 4]);
    let opacity_byte = (f64::from(opacity) * 255.0).round().clamp(0.0, 255.0) as u8;
    for ((destination, source), mask) in pixels
        .chunks_exact_mut(4)
        .zip(layer.data().chunks_exact(4))
        .zip(clip.chunks_exact(4))
    {
        if source[3] != 0 && mask[3] != 0 {
            let mut source: [u8; 4] = source.try_into().unwrap();
            scale_premultiplied(&mut source, mask[3]);
            scale_premultiplied(&mut source, opacity_byte);
            blend_premultiplied_pixel(destination, source);
        }
    }
    true
}

// The direct layer's integer crop is an exact viewport clip only when its
// transformed edges lie on device pixels. Otherwise the shared alpha-mask
// path above evaluates fractional/rotated edges with coverage.
fn svg_viewport_has_integer_device_edges(image: &usvg::Image, scale: f32) -> bool {
    let transform = image.abs_transform();
    let bounds = image.abs_bounding_box();
    transform.kx == 0.0
        && transform.ky == 0.0
        && [bounds.left(), bounds.top(), bounds.right(), bounds.bottom()]
            .into_iter()
            .all(|coordinate| {
                let device = f64::from(coordinate) * f64::from(scale);
                device.is_finite() && device.fract() == 0.0
            })
}

#[path = "svg_clip_reference.rs"]
mod svg_clip_reference;
use svg_clip_reference::{flatten_tolerance64, intrinsic_clip_paint_bounds};

fn intrinsic_group_clip<'a>(
    group: &usvg::Group,
    paths: Option<&'a [SvgPath]>,
) -> Option<(&'a SvgPath, &'a crate::svg_precision::IntrinsicClip)> {
    paths?.iter().find_map(|path| {
        path.intrinsic_clips
            .iter()
            .find(|clip| clip.id == group.id())
            .map(|clip| (path, clip))
    })
}

// Original <image> has no children. usvg inserts anonymous aspect/placement
// groups below its identity; the retained original box sampler performs that
// generated clipping. Authored image effects never get this qualification.
fn sole_png_image(group: &usvg::Group) -> Option<&usvg::Image> {
    if group.mask().is_some()
        || !group.filters().is_empty()
        || group.isolate()
        || group.blend_mode() != usvg::BlendMode::Normal
        || group.children().len() != 1
    {
        return None;
    }
    match &group.children()[0] {
        usvg::Node::Image(image) if matches!(image.kind(), usvg::ImageKind::PNG(_)) => Some(image),
        usvg::Node::Group(child) if child.id().is_empty() && child.opacity().get() == 1.0 => {
            sole_png_image(child)
        }
        _ => None,
    }
}
fn retained_png_wrapper<'a>(
    group: &usvg::Group,
    by_id: Option<&'a HashMap<String, SvgPath>>,
) -> Option<&'a SvgPath> {
    let source = by_id?.get(group.id())?;
    (source.raster_image.is_some()
        && source.scene_transform.is_some()
        && source.intrinsic_transforms.is_some()
        && source.image_viewport.is_some()
        && sole_png_image(group).is_some())
    .then_some(source)
}

// usvg transfers an image element's identity to its generated placement
// group. Bind only a source image carrier, never an arbitrary group/path ID.
fn retained_raster_image<'a>(
    group: &usvg::Group,
    image: &usvg::Image,
    by_id: Option<&'a HashMap<String, SvgPath>>,
) -> Option<&'a SvgPath> {
    let sources = by_id?;
    sources
        .get(image.id())
        .or_else(|| sources.get(group.id()))
        .filter(|source| source.raster_image.is_some())
}

fn direct_svg_group_supported(
    group: &usvg::Group,
    paths: Option<&[SvgPath]>,
    by_id: Option<&HashMap<String, SvgPath>>,
) -> bool {
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("D2_RS_TRACE_SVG_RAW_PATHS").is_some() {
        for child in group.children() {
            if let usvg::Node::Image(image) = child {
                eprintln!(
                    "d2-rs-svg-raster-support group={} image={} clip={} isolate={} opacity={} carrier={:?}",
                    group.id(),
                    image.id(),
                    group.clip_path().is_some(),
                    group.isolate(),
                    group.opacity().get(),
                    retained_raster_image(group, image, by_id).map(|s| (
                        s.raster_image.is_some(),
                        s.scene_transform.is_some(),
                        s.intrinsic_transforms.is_some()
                    ))
                );
            }
        }
    }
    if retained_png_wrapper(group, by_id).is_some() {
        return true;
    }
    (group.clip_path().is_none() || intrinsic_group_clip(group, paths).is_some())
        && group.mask().is_none()
        && group.filters().is_empty()
        && !group.isolate()
        && group.blend_mode() == usvg::BlendMode::Normal
        && group.children().iter().all(|child| match child {
            usvg::Node::Group(group) => direct_svg_group_supported(group, paths, by_id),
            usvg::Node::Path(_) => true,
            usvg::Node::Image(image) => {
                !image.is_visible()
                    || (matches!(image.kind(), usvg::ImageKind::PNG(_))
                        && retained_raster_image(group, image, by_id).is_some_and(|source| {
                            source.raster_image.is_some()
                                && source.scene_transform.is_some()
                                && source.intrinsic_transforms.is_some()
                                && source.image_viewport.is_some()
                        }))
            }
            usvg::Node::Text(_) => false,
        })
}

fn svg_group_path_count(group: &usvg::Group) -> usize {
    group
        .children()
        .iter()
        .map(|child| match child {
            usvg::Node::Group(group) => svg_group_path_count(group),
            usvg::Node::Path(path) => {
                usize::from(path.is_visible() && (path.fill().is_some() || path.stroke().is_some()))
            }
            usvg::Node::Image(_) | usvg::Node::Text(_) => 0,
        })
        .sum()
}

fn direct_svg_group_layer_bounds(
    image: &usvg::Image,
    tree: &Tree,
    transform: Transform,
    scale: f32,
    canvas_width: usize,
    canvas_height: usize,
    by_id: Option<&HashMap<String, SvgPath>>,
) -> Option<(usize, usize, usize, usize)> {
    let viewport = image.abs_bounding_box();
    let viewport_left = (f64::from(viewport.left()) * f64::from(scale)).floor();
    let viewport_top = (f64::from(viewport.top()) * f64::from(scale)).floor();
    let viewport_right = (f64::from(viewport.right()) * f64::from(scale)).ceil();
    let viewport_bottom = (f64::from(viewport.bottom()) * f64::from(scale)).ceil();

    // The Go renderer reserves one device pixel around vector ink for
    // antialiasing and path-flattening tolerance before intersecting the
    // resulting bounds with the image viewport.
    // Source prepared ink is the union of actual painted primitives. Empty
    // SVG groups have no ink; usvg's aggregate box can include their origin.
    let mut ink_corners = Vec::new();
    fn painted_corners(group: &usvg::Group, points: &mut Vec<(f32, f32)>) {
        for child in group.children() {
            match child {
                usvg::Node::Group(group) => painted_corners(group, points),
                usvg::Node::Path(path)
                    if path.is_visible() && (path.fill().is_some() || path.stroke().is_some()) =>
                {
                    let ink = path.abs_stroke_bounding_box();
                    points.extend([
                        (ink.left(), ink.top()),
                        (ink.right(), ink.top()),
                        (ink.right(), ink.bottom()),
                        (ink.left(), ink.bottom()),
                    ]);
                }
                _ => {}
            }
        }
    }
    painted_corners(tree.root(), &mut ink_corners);
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (x, y) in ink_corners {
        let mut point = Point::from_xy(x, y);
        transform.map_point(&mut point);
        let x = f64::from(point.x) * f64::from(scale);
        let y = f64::from(point.y) * f64::from(scale);
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    // Original image bounds use Image.Box in the complete original matrix,
    // independently from aspect placement and from the vector ink inventory.
    fn raster_corners(
        group: &usvg::Group,
        by_id: Option<&HashMap<String, SvgPath>>,
        parent: Transform,
        scale: f32,
        output: &mut Vec<Point64>,
    ) {
        if let Some(source) = retained_png_wrapper(group, by_id) {
            if let Some(matrix) =
                intrinsic_svg_transform(source, parent, scale, Point64 { x: 0.0, y: 0.0 })
            {
                let [x, y, w, h] = source.raster_image.unwrap().image_box;
                output.extend(
                    [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
                        .map(|(x, y)| map_point(Point64 { x, y }, matrix)),
                );
            }
            return;
        }
        for child in group.children() {
            match child {
                usvg::Node::Group(group) => raster_corners(group, by_id, parent, scale, output),
                usvg::Node::Image(image) if image.is_visible() => {
                    if let Some(source) = retained_raster_image(group, image, by_id) {
                        if let (Some(raster), Some(matrix)) = (
                            source.raster_image,
                            intrinsic_svg_transform(
                                source,
                                parent,
                                scale,
                                Point64 { x: 0.0, y: 0.0 },
                            ),
                        ) {
                            let [x, y, w, h] = raster.image_box;
                            output.extend(
                                [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
                                    .map(|(x, y)| map_point(Point64 { x, y }, matrix)),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut raster_points = Vec::new();
    raster_corners(tree.root(), by_id, transform, scale, &mut raster_points);
    for point in raster_points {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    if !min_x.is_finite() {
        return None;
    }
    let left = (min_x - 1.0).floor().max(viewport_left).max(0.0);
    let top = (min_y - 1.0).floor().max(viewport_top).max(0.0);
    let right = (max_x + 1.0)
        .ceil()
        .min(viewport_right)
        .min(canvas_width as f64);
    let bottom = (max_y + 1.0)
        .ceil()
        .min(viewport_bottom)
        .min(canvas_height as f64);
    if right <= left || bottom <= top {
        return None;
    }
    Some((left as usize, top as usize, right as usize, bottom as usize))
}

fn path_uses_analytic_paint(path: &usvg::Path) -> bool {
    path.fill().is_some_and(|fill| {
        matches!(
            fill.paint(),
            Paint::LinearGradient(_) | Paint::RadialGradient(_) | Paint::Pattern(_)
        )
    }) || path
        .stroke()
        .is_some_and(|stroke| matches!(stroke.paint(), Paint::LinearGradient(_)))
}

fn direct_svg_group_fits_viewport(tree: &Tree) -> bool {
    let bounds = tree.root().abs_stroke_bounding_box();
    let size = tree.size();
    let epsilon = 1e-4;
    f64::from(bounds.left()) >= -epsilon
        && f64::from(bounds.top()) >= -epsilon
        && f64::from(bounds.right()) <= f64::from(size.width()) + epsilon
        && f64::from(bounds.bottom()) <= f64::from(size.height()) + epsilon
}

fn draw_svg_group_direct(
    group: &usvg::Group,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    text_outline: bool,
    parent_transform: Transform,
    device_origin: Point64,
    raster_sources: &RasterSourceMap<'_>,
    raw_paths: Option<&[SvgPath]>,
    raw_paths_by_id: Option<&HashMap<String, SvgPath>>,
    group_opacities: Option<&HashMap<String, f64>>,
) -> bool {
    let mut path_index = 0;
    let paint_reference = TextPaintBounds {
        left: device_origin.x as i64,
        top: device_origin.y as i64,
        right: device_origin.x as i64 + width as i64,
        bottom: device_origin.y as i64 + height as i64,
    };
    draw_svg_group_direct_impl(
        group,
        pixels,
        width,
        height,
        scale,
        text_outline,
        parent_transform,
        device_origin,
        raster_sources,
        raw_paths,
        raw_paths_by_id,
        group_opacities,
        &mut path_index,
        paint_reference,
    )
}

fn draw_svg_group_direct_impl(
    group: &usvg::Group,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    text_outline: bool,
    parent_transform: Transform,
    device_origin: Point64,
    raster_sources: &RasterSourceMap<'_>,
    raw_paths: Option<&[SvgPath]>,
    raw_paths_by_id: Option<&HashMap<String, SvgPath>>,
    group_opacities: Option<&HashMap<String, f64>>,
    path_index: &mut usize,
    paint_reference: TextPaintBounds,
) -> bool {
    let retained_clip = intrinsic_group_clip(group, raw_paths);
    let paint_reference = if retained_clip.is_some() {
        intrinsic_clip_paint_bounds(
            group,
            parent_transform,
            scale,
            raw_paths,
            raw_paths_by_id,
            *path_index,
            paint_reference,
        )
        .unwrap_or(paint_reference)
    } else {
        paint_reference
    };
    if group.opacity().get() == 1.0 && retained_clip.is_none() {
        return draw_svg_group_direct_children(
            group,
            pixels,
            width,
            height,
            scale,
            text_outline,
            parent_transform,
            device_origin,
            raster_sources,
            raw_paths,
            raw_paths_by_id,
            group_opacities,
            path_index,
            paint_reference,
        );
    }
    // Source retains a group layer and applies opacity after its children
    // have composited, rather than multiplying each child's paint alpha.
    let mut layer = vec![0_u8; pixels.len()];
    if !draw_svg_group_direct_children(
        group,
        &mut layer,
        width,
        height,
        scale,
        text_outline,
        parent_transform,
        device_origin,
        raster_sources,
        raw_paths,
        raw_paths_by_id,
        group_opacities,
        path_index,
        paint_reference,
    ) {
        return false;
    }
    if let Some((path, clip)) = retained_clip {
        // Original compile.go: apply the nested viewport clip after its
        // complete child layer is composited, before group opacity.
        let parent = parent_transform.post_scale(scale, scale);
        let mut transform = path.scene_transform.map_or_else(
            || canonical_transform(parent),
            |m| {
                let scale = f64::from(scale);
                Affine64 {
                    sx: m[0] * scale,
                    ky: m[1] * scale,
                    kx: m[2] * scale,
                    sy: m[3] * scale,
                    tx: m[4] * scale,
                    ty: m[5] * scale,
                }
            },
        );
        for matrix in &clip.transforms {
            transform = source_affine_mul(
                transform,
                Affine64 {
                    sx: matrix[0],
                    ky: matrix[1],
                    kx: matrix[2],
                    sy: matrix[3],
                    tx: matrix[4],
                    ty: matrix[5],
                },
            );
        }
        let reference_origin = Point64 {
            x: paint_reference.left as f64,
            y: paint_reference.top as f64,
        };
        transform.tx -= reference_origin.x;
        transform.ty -= reference_origin.y;
        let points = vec![
            Point64 { x: 0.0, y: 0.0 },
            Point64 {
                x: clip.size[0],
                y: 0.0,
            },
            Point64 {
                x: clip.size[0],
                y: clip.size[1],
            },
            Point64 {
                x: 0.0,
                y: clip.size[1],
            },
        ];
        let mapped = map_centerline((points, true), transform);
        let mut rasterizer = Rasterizer::new(width, height, 0.375);
        rasterizer.add_fill_polyline_with_origin_offset(
            &mapped.0,
            Point64 {
                x: reference_origin.x - device_origin.x,
                y: reference_origin.y - device_origin.y,
            },
        );
        let mut mask = vec![0_u8; pixels.len()];
        rasterizer.paint(&mut mask, [255; 4]);
        for (pixel, coverage) in layer.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
            scale_premultiplied(pixel, coverage[3]);
        }
    }
    let parsed_opacity = group.opacity().get();
    let opacity = group_opacities
        .and_then(|values| values.get(group.id()))
        .copied()
        .filter(|value| (*value as f32).to_bits() == parsed_opacity.to_bits())
        .unwrap_or_else(|| f64::from(parsed_opacity));
    let alpha = (opacity * 255.0).round() as u8;
    for (destination, source) in pixels.chunks_exact_mut(4).zip(layer.chunks_exact_mut(4)) {
        scale_premultiplied(source, alpha);
        blend_premultiplied_pixel(destination, source.try_into().unwrap());
    }
    true
}

fn draw_svg_group_direct_children(
    group: &usvg::Group,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    text_outline: bool,
    parent_transform: Transform,
    device_origin: Point64,
    raster_sources: &RasterSourceMap<'_>,
    raw_paths: Option<&[SvgPath]>,
    raw_paths_by_id: Option<&HashMap<String, SvgPath>>,
    group_opacities: Option<&HashMap<String, f64>>,
    path_index: &mut usize,
    paint_reference: TextPaintBounds,
) -> bool {
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("D2_RS_TRACE_SVG_RAW_PATHS").is_some() {
        eprintln!(
            "d2-rs-svg-group id={} children={}",
            group.id(),
            group.children().len()
        );
    }
    if let Some(source) = retained_png_wrapper(group, raw_paths_by_id) {
        let image = sole_png_image(group).unwrap();
        let usvg::ImageKind::PNG(data) = image.kind() else {
            return false;
        };
        let mut raster = source.raster_image.unwrap();
        let Some(matrix) = intrinsic_svg_transform(source, parent_transform, scale, device_origin)
        else {
            return false;
        };
        raster.matrix = [
            matrix.sx, matrix.ky, matrix.kx, matrix.sy, matrix.tx, matrix.ty,
        ];
        return crate::png_image::draw(image, data, pixels, width, height, 1.0, 1.0, Some(&raster));
    }
    for child in group.children() {
        let ok = match child {
            usvg::Node::Group(group) => draw_svg_group_direct_impl(
                group,
                pixels,
                width,
                height,
                scale,
                text_outline,
                parent_transform,
                device_origin,
                raster_sources,
                raw_paths,
                raw_paths_by_id,
                group_opacities,
                path_index,
                paint_reference,
            ),
            usvg::Node::Path(path) => {
                // Original SVG import emits no primitive for hidden/unpainted paths.
                // These retained usvg nodes consume neither ink nor raw-path ordinals.
                if !path.is_visible() || (path.fill().is_none() && path.stroke().is_none()) {
                    continue;
                }
                let source_path = raw_paths_by_id
                    .and_then(|paths| paths.get(path.id()))
                    .or_else(|| raw_paths.and_then(|paths| paths.get(*path_index)));
                #[cfg(feature = "diagnostic-traces")]
                if std::env::var_os("D2_RS_TRACE_SVG_RAW_PATHS").is_some()
                    && raw_paths_by_id.is_some_and(|p| !p.is_empty())
                {
                    eprintln!(
                        "d2-rs-svg-id-bind path={} keys={:?} found={}",
                        path.id(),
                        raw_paths_by_id.unwrap().keys().collect::<Vec<_>>(),
                        source_path.is_some()
                    );
                }
                *path_index += 1;
                #[cfg(feature = "diagnostic-traces")]
                if source_path.is_some() && std::env::var_os("D2_RS_TRACE_SVG_RAW_PATHS").is_some()
                {
                    let retained = source_path.unwrap();
                    eprintln!(
                        "d2-rs-svg-retained scene={:?} chain={:?} fill={:?} stroke={:?}",
                        retained.scene_transform,
                        retained.intrinsic_transforms,
                        path.fill(),
                        path.stroke()
                    );
                    let path_transform = path.abs_transform();
                    eprintln!(
                        "d2-rs-svg-raw-path id={} index={} path=({:.9},{:.9},{:.9},{:.9},{:.9},{:.9}) parent=({:.9},{:.9},{:.9},{:.9},{:.9},{:.9})",
                        path.id(),
                        *path_index - 1,
                        path_transform.sx,
                        path_transform.kx,
                        path_transform.ky,
                        path_transform.sy,
                        path_transform.tx,
                        path_transform.ty,
                        parent_transform.sx,
                        parent_transform.kx,
                        parent_transform.ky,
                        parent_transform.sy,
                        parent_transform.tx,
                        parent_transform.ty,
                    );
                }
                source_path
                    .and_then(|source_path| {
                        draw_raw_svg_path(
                            source_path,
                            path,
                            pixels,
                            width,
                            height,
                            scale,
                            parent_transform,
                            device_origin,
                            Point64 {
                                x: paint_reference.left as f64,
                                y: paint_reference.top as f64,
                            },
                            raster_sources,
                        )
                    })
                    .unwrap_or_else(|| {
                        draw_path_with_transform(
                            path,
                            pixels,
                            width,
                            height,
                            scale,
                            1.0,
                            text_outline,
                            Some(parent_transform),
                            Point64 { x: 0.0, y: 0.0 },
                            Point64 { x: 0.0, y: 0.0 },
                            path_uses_analytic_paint(path),
                            raster_sources,
                            device_origin,
                            source_path,
                        )
                    })
            }
            usvg::Node::Image(image) => {
                if !image.is_visible() {
                    continue;
                }
                match (
                    image.kind(),
                    retained_raster_image(group, image, raw_paths_by_id),
                ) {
                    (usvg::ImageKind::PNG(data), Some(source)) => {
                        if let (Some(mut raster), Some(matrix)) = (
                            source.raster_image,
                            intrinsic_svg_transform(source, parent_transform, scale, device_origin),
                        ) {
                            raster.matrix = [
                                matrix.sx, matrix.ky, matrix.kx, matrix.sy, matrix.tx, matrix.ty,
                            ];
                            crate::png_image::draw(
                                image,
                                data,
                                pixels,
                                width,
                                height,
                                1.0,
                                1.0,
                                Some(&raster),
                            )
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }
            usvg::Node::Text(_) => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

// Original scene path preparation keeps the complete f64 transform for fills
// and strokes. The parsed usvg matrix remains only the unqualified fallback.
fn intrinsic_svg_transform(
    source: &SvgPath,
    parent: Transform,
    scale: f32,
    origin: Point64,
) -> Option<Affine64> {
    let chain = source.intrinsic_transforms.as_ref()?;
    let mut result = source.scene_transform.map_or_else(
        || canonical_transform(parent.post_scale(scale, scale)),
        |m| {
            let scale = f64::from(scale);
            Affine64 {
                sx: m[0] * scale,
                ky: m[1] * scale,
                kx: m[2] * scale,
                sy: m[3] * scale,
                tx: m[4] * scale,
                ty: m[5] * scale,
            }
        },
    );
    for matrix in chain {
        result = source_affine_mul(
            result,
            Affine64 {
                sx: matrix[0],
                ky: matrix[1],
                kx: matrix[2],
                sy: matrix[3],
                tx: matrix[4],
                ty: matrix[5],
            },
        );
    }
    result.tx -= origin.x;
    result.ty -= origin.y;
    Some(result)
}

fn draw_raw_svg_path(
    source_path: &SvgPath,
    path: &usvg::Path,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    parent_transform: Transform,
    device_origin: Point64,
    reference_origin: Point64,
    raster_sources: &RasterSourceMap<'_>,
) -> Option<bool> {
    let fill = path.fill()?;
    if path.stroke().is_some()
        || fill.rule() != FillRule::NonZero
        || !matches!(
            fill.paint(),
            Paint::Color(_) | Paint::LinearGradient(_) | Paint::RadialGradient(_)
        )
    {
        return None;
    }
    let scene_transform = path_scene_transform(path.abs_transform(), Some(parent_transform));
    let device_transform = scene_transform.post_scale(scale, scale);
    let device_transform = Transform::from_row(
        device_transform.sx,
        device_transform.ky,
        device_transform.kx,
        device_transform.sy,
        device_transform.tx - device_origin.x as f32,
        device_transform.ty - device_origin.y as f32,
    );
    let transform = intrinsic_svg_transform(source_path, parent_transform, scale, reference_origin)
        .unwrap_or_else(|| canonical_transform(device_transform));
    let source_tolerance = if source_path.intrinsic_transforms.is_some() {
        flatten_tolerance64(transform)
    } else {
        flatten_tolerance(device_transform)
    };
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("D2_RS_TRACE_SVG_RAW_PATHS").is_some() {
        eprintln!(
            "d2-rs-svg-f64 transform={:?} tolerance={source_tolerance:.17e} origin={:?} subpaths={:?}",
            (
                transform.sx,
                transform.ky,
                transform.kx,
                transform.sy,
                transform.tx,
                transform.ty
            ),
            (device_origin.x, device_origin.y),
            collect_raw_subpaths(source_path, source_tolerance, transform).map(|paths| paths
                .into_iter()
                .map(|(points, closed)| (
                    points.into_iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(),
                    closed
                ))
                .collect::<Vec<_>>())
        );
    }
    let mut rasterizer = Rasterizer::new(width, height, 0.375);
    rasterizer.set_trace_origin(device_origin);
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("D2_RS_TRACE_SVG_RAW_PATHS").is_some() {
        eprintln!(
            "d2-rs-svg-reference reference=({},{}) device=({},{})",
            reference_origin.x, reference_origin.y, device_origin.x, device_origin.y
        );
    }
    for (points, closed) in collect_raw_subpaths(source_path, source_tolerance, transform)? {
        let mapped = map_centerline((points, closed), transform);
        rasterizer.add_fill_polyline_with_origin_offset(
            &mapped.0,
            Point64 {
                x: reference_origin.x - device_origin.x,
                y: reference_origin.y - device_origin.y,
            },
        );
    }
    let exact_color = solid_paint_exact(
        fill.paint(),
        fill.opacity().get(),
        source_path.paint_opacities.map(|o| o[0]),
    );
    let sample = paint_sampler(
        fill.paint(),
        fill.opacity().get(),
        device_transform,
        &raster_sources.pattern_tiles,
        Some(transform),
        None,
        device_origin,
        &raster_sources.gradient_stops,
    );
    let linear_stops = match fill.paint() {
        Paint::LinearGradient(gradient) => raster_sources.gradient_stops.get(gradient.id()),
        _ => None,
    };
    let paint = |x, y| {
        exact_color.or_else(|| {
            if let Paint::LinearGradient(gradient) = fill.paint() {
                if source_path.scene_transform.is_some()
                    && source_path.intrinsic_transforms.is_some()
                {
                    let mut global_transform = transform;
                    global_transform.tx += reference_origin.x;
                    global_transform.ty += reference_origin.y;
                    return linear_gradient_color_at64(
                        gradient,
                        fill.opacity().get(),
                        global_transform,
                        x,
                        y,
                        device_origin,
                        linear_stops,
                    );
                }
            }
            sample(x, y)
        })
    };
    if matches!(
        fill.paint(),
        Paint::LinearGradient(_) | Paint::RadialGradient(_)
    ) {
        // Original analytic gradient painting uses compositeNRGBAOverRGBA,
        // unlike the standard solid-color scanline Draw kernel.
        rasterizer.paint_with_go_blend(pixels, paint);
    } else {
        rasterizer.paint_with(pixels, paint);
    }
    Some(true)
}

/// Source renderEffectNode applies the mask before blending the retained layer
/// with the backdrop. Blending into a transparent temporary layer first loses
/// the backdrop and cannot reproduce darken/burn/overlay/lighten.
fn draw_masked_blend_group(
    tree: &Tree,
    group: &usvg::Group,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    inherited_opacity: f32,
    text_outline: bool,
    raster_sources: &RasterSourceMap<'_>,
    svg_image_paths: &SvgImagePathSources,
) -> bool {
    let Some(mask) = group.mask() else {
        return false;
    };
    let mut layer = take_effect_layer(pixels.len());
    let scope = LayerPaintScope::begin(tree, group, &layer);
    let origin = group_reference_origin_with_text_transform(group, scale, true);
    if !draw_group_children(
        tree,
        group,
        &mut layer,
        width,
        height,
        scale,
        inherited_opacity,
        text_outline,
        false,
        origin,
        true,
        Some(origin),
        raster_sources,
        svg_image_paths,
    ) {
        return false;
    }
    let region = scope.finish(width, height);
    debug_assert_layer_in_region(&layer, width, &region);
    // The mask only matters where the layer received paint.
    let Some(alpha) = render_mask_alpha(
        mask,
        group.abs_transform(),
        width,
        height,
        &region,
        scale,
        raster_sources.device_origin,
        svg_image_paths
            .native_connection_mask(mask.id())
            .map(|native| {
                (
                    native,
                    raster_sources.native_canvas_size(width, height),
                    svg_image_paths.native_mask_transform(group.id()),
                )
            }),
    ) else {
        return false;
    };
    for (y, x0, x1, offset) in region.rows_in(0, height) {
        let row = &mut layer[(y * width + x0) * 4..(y * width + x1) * 4];
        let row_alpha = &alpha[offset..offset + (x1 - x0)];
        for (pixel, &coverage) in row.chunks_exact_mut(4).zip(row_alpha) {
            scale_premultiplied(pixel, coverage);
        }
    }
    composite_effect_group(
        pixels,
        &layer,
        width,
        height,
        effect_composite_bounds(group, scale, raster_sources.device_origin, width, height),
        &region,
        scale,
        svg_image_paths.exact_effect_opacity(group.id(), group.opacity().get()),
        group.blend_mode(),
    );
    recycle_effect_layer(layer, width, height, region);
    true
}

/// Debug builds check `LayerPaintScope`: a layer has no paint outside it.
fn debug_assert_layer_in_region(layer: &[u8], width: usize, region: &LayerRegion) {
    if cfg!(debug_assertions) && region.tracked && !go_band_recording() {
        for (index, pixel) in layer.chunks_exact(4).enumerate() {
            let (x, y) = (index % width, index / width);
            assert!(
                pixel[3] == 0 || region.contains(x, y),
                "effect layer paint at ({x}, {y}) lies outside its region"
            );
        }
    }
}

fn draw_masked_group(
    tree: &Tree,
    group: &usvg::Group,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    text_outline: bool,
    raster_sources: &RasterSourceMap<'_>,
    svg_image_paths: &SvgImagePathSources,
) -> bool {
    let Some(mask) = group.mask() else {
        return false;
    };
    let storage_region = masked_path_region(group, width, height);
    let mut layer = take_effect_layer(
        storage_region
            .as_ref()
            .map_or(pixels.len(), |region| region.len * 4),
    );
    let output_scope = storage_region
        .as_ref()
        .map(|region| LayerOutputScope::new(&layer, region.clone()));
    let scope = LayerPaintScope::begin(tree, group, &layer);
    if !draw_group_impl(
        tree,
        group,
        &mut layer,
        width,
        height,
        scale,
        opacity,
        text_outline,
        true,
        None,
        true,
        None,
        raster_sources,
        svg_image_paths,
    ) {
        return false;
    }
    let region = scope.finish(width, height);
    drop(output_scope);
    if let Some(storage) = &storage_region {
        if cfg!(debug_assertions) {
            for (y, x0, x1, offset) in storage.rows_in(0, height) {
                for (x, pixel) in (x0..x1).zip(layer[offset * 4..].chunks_exact(4)) {
                    assert!(pixel[3] == 0 || region.contains(x, y));
                }
            }
        }
    } else {
        debug_assert_layer_in_region(&layer, width, &region);
    }
    // The mask only matters where the layer received paint.
    let Some(mask_alpha) = render_mask_alpha(
        mask,
        group.abs_transform(),
        width,
        height,
        &region,
        scale,
        raster_sources.device_origin,
        svg_image_paths
            .native_connection_mask(mask.id())
            .map(|native| {
                (
                    native,
                    raster_sources.native_canvas_size(width, height),
                    svg_image_paths.native_mask_transform(group.id()),
                )
            }),
    ) else {
        return false;
    };
    #[cfg(feature = "diagnostic-traces")]
    let trace_pixel = raster_trace_pixel();
    #[cfg(feature = "diagnostic-traces")]
    let skip_transparent_blocks = trace_pixel.is_none();
    #[cfg(not(feature = "diagnostic-traces"))]
    let skip_transparent_blocks = true;
    // Pixels outside the region are transparent, which composites nothing.
    for (y, x0, x1, alpha_row) in region.rows_in(0, height) {
        let row = y * width + x0..y * width + x1;
        let source_start = storage_region.as_ref().map_or(row.start, |storage| {
            let (left, _, offset) = storage.row(y as isize).expect("painted storage row");
            offset + x0 - left
        });
        let mut next = row.start;
        while next < row.end {
            let source_index = source_start + next - row.start;
            // Source compositeLayerOver skips uniform transparent source blocks.
            // A mask cannot make a transparent source contribute any paint.
            if skip_transparent_blocks
                && row.end - next >= 64
                && layer[source_index * 4 + 3] == 0
                && layer[(source_index + 63) * 4 + 3] == 0
                && layer[source_index * 4..(source_index + 64) * 4]
                    .chunks_exact(4)
                    .fold(0, |alpha, pixel| alpha | pixel[3])
                    == 0
            {
                next += 64;
                continue;
            }
            let index = next;
            next += 1;
            let alpha = mask_alpha[alpha_row + index - row.start];
            #[cfg(feature = "diagnostic-traces")]
            let trace_this_pixel = trace_pixel == Some((index % width, index / width));
            if alpha == 0 {
                #[cfg(feature = "diagnostic-traces")]
                if trace_this_pixel {
                    eprintln!(
                        "d2-rs-raster mask-composite x={} y={} mask_alpha=0 skipped=true",
                        index % width,
                        index / width
                    );
                }
                continue;
            }
            let pixel_index = index * 4;
            let source = &mut layer[source_index * 4..source_index * 4 + 4];
            #[cfg(feature = "diagnostic-traces")]
            let source_before =
                trace_this_pixel.then(|| [source[0], source[1], source[2], source[3]]);
            scale_premultiplied(source, alpha);
            #[cfg(feature = "diagnostic-traces")]
            let source_after =
                trace_this_pixel.then(|| [source[0], source[1], source[2], source[3]]);
            #[cfg(feature = "diagnostic-traces")]
            let destination_before = trace_this_pixel.then(|| {
                let pixel = &pixels[pixel_index..pixel_index + 4];
                [pixel[0], pixel[1], pixel[2], pixel[3]]
            });
            blend_premultiplied_pixel(
                &mut pixels[pixel_index..pixel_index + 4],
                [source[0], source[1], source[2], source[3]],
            );
            #[cfg(feature = "diagnostic-traces")]
            if let (Some(source_before), Some(source_after), Some(destination_before)) =
                (source_before, source_after, destination_before)
            {
                let destination_after = &pixels[pixel_index..pixel_index + 4];
                eprintln!(
                    "d2-rs-raster mask-composite x={} y={} mask_alpha={alpha} source_before={source_before:?} source_after={source_after:?} destination_before={destination_before:?} destination_after={:?}",
                    index % width,
                    index / width,
                    [
                        destination_after[0],
                        destination_after[1],
                        destination_after[2],
                        destination_after[3]
                    ],
                );
            }
        }
    }
    note_painted(pixels, &region);
    recycle_effect_layer(layer, width, height, region);
    true
}

/// Mask coverage for the pixels of `region`, in its compact layout.
fn render_mask_alpha(
    mask: &usvg::Mask,
    parent_transform: Transform,
    width: usize,
    height: usize,
    region: &std::rc::Rc<LayerRegion>,
    scale: f32,
    device_origin: Point64,
    native: Option<(
        &crate::svg_precision::NativeConnectionMask,
        (usize, usize),
        [f64; 6],
    )>,
) -> Option<ZeroedLayer> {
    // Mask content is not part of the masked node's primitive bounds (Go
    // only intersects node.bounds with the mask root's bounds), and a band
    // record pass composites nothing, so it needs no coverage.
    if go_band_recording() {
        return Some(ZeroedLayer::new(region.len));
    }
    // D2 gives every labeled connection the same full-diagram mask. Without
    // band windows a mask's coverage does not depend on the masked layer, so
    // render it once for the whole surface and copy each layer's region.
    if region.len != width * height && go_band_windows(width, height).is_none() {
        let key = MaskKey {
            mask: mask as *const usvg::Mask as usize,
            native: native.map_or((0, (0, 0), [0; 6]), |(native, canvas, local)| {
                (native as *const _ as usize, canvas, local.map(f64::to_bits))
            }),
            transform: [
                parent_transform.sx,
                parent_transform.ky,
                parent_transform.kx,
                parent_transform.sy,
                parent_transform.tx,
                parent_transform.ty,
                scale,
            ]
            .map(f32::to_bits),
            origin: [device_origin.x.to_bits(), device_origin.y.to_bits()],
            size: (width, height),
            rows: frame_band_rows(width, height).unwrap_or((0, height)),
        };
        // A mask used once is cheaper to render for its region alone; the
        // second use of the same mask renders and keeps the whole surface.
        let cached = MASK_CACHE.with(|cache| cache.borrow_mut().lookup(&key));
        let cached = match cached {
            MaskLookup::Hit(full) => Some(full),
            MaskLookup::Repeat => None,
            MaskLookup::First => {
                return render_mask_alpha_in(
                    mask,
                    parent_transform,
                    width,
                    height,
                    region,
                    scale,
                    device_origin,
                    native,
                );
            }
        };
        let full = match cached {
            Some(full) => full,
            None => {
                // Render in row strips so the RGBA coverage scratch stays
                // strip-sized; each strip's coverage is exact (see
                // Rasterizer::set_output_region).
                let mut full = ZeroedLayer::new(width * height);
                let (first_row, end_row) = key.rows;
                for top in (first_row..end_row).step_by(PAINT_BUCKET_ROWS) {
                    let bottom = (top + PAINT_BUCKET_ROWS).min(end_row);
                    let strip = std::rc::Rc::new(LayerRegion::from_rects(
                        [(0, top as isize, width as isize, bottom as isize)],
                        width,
                        height,
                    ));
                    let alpha = render_mask_alpha_in(
                        mask,
                        parent_transform,
                        width,
                        height,
                        &strip,
                        scale,
                        device_origin,
                        native,
                    )?;
                    full[top * width..bottom * width].copy_from_slice(&alpha);
                }
                let full = std::rc::Rc::new(full);
                MASK_CACHE.with(|cache| cache.borrow_mut().insert(key, full.clone()));
                full
            }
        };
        let mut alpha = ZeroedLayer::new(region.len);
        for (y, x0, x1, offset) in region.rows_in(0, height) {
            alpha[offset..offset + (x1 - x0)]
                .copy_from_slice(&full[y * width + x0..y * width + x1]);
        }
        return Some(alpha);
    }
    render_mask_alpha_in(
        mask,
        parent_transform,
        width,
        height,
        region,
        scale,
        device_origin,
        native,
    )
}

#[derive(Clone, PartialEq)]
struct MaskKey {
    mask: usize,
    // Native connection mask and the frame size its viewport maps onto.
    native: (usize, (usize, usize), [u64; 6]),
    transform: [u32; 7],
    origin: [u64; 2],
    size: (usize, usize),
    // Rows rendered: the current band pass, or the whole surface.
    rows: (usize, usize),
}

/// Whole-surface mask coverage kept for reuse within one render.
#[derive(Default)]
struct MaskCache {
    seen: Vec<MaskKey>,
    entries: Vec<(MaskKey, std::rc::Rc<ZeroedLayer>)>,
}

enum MaskLookup {
    Hit(std::rc::Rc<ZeroedLayer>),
    Repeat,
    First,
}

impl MaskCache {
    const MAX_BYTES: usize = 64 << 20;
    const MAX_SEEN: usize = 64;

    fn lookup(&mut self, key: &MaskKey) -> MaskLookup {
        if let Some((_, full)) = self.entries.iter().find(|(cached, _)| cached == key) {
            return MaskLookup::Hit(full.clone());
        }
        if self.seen.contains(key) {
            return MaskLookup::Repeat;
        }
        if self.seen.len() == Self::MAX_SEEN {
            self.seen.remove(0);
        }
        self.seen.push(key.clone());
        MaskLookup::First
    }

    fn insert(&mut self, key: MaskKey, full: std::rc::Rc<ZeroedLayer>) {
        // Bands are drawn in order: coverage kept for an earlier band (or
        // band origin) is never requested again.
        self.entries
            .retain(|(cached, _)| cached.origin == key.origin && cached.rows == key.rows);
        let mut bytes = full.len() + self.entries.iter().map(|(_, e)| e.len()).sum::<usize>();
        while bytes > Self::MAX_BYTES && !self.entries.is_empty() {
            bytes -= self.entries.remove(0).1.len();
        }
        self.entries.push((key, full));
    }
}

/// Lend `visit` a zeroed RGBA coverage scratch of `len` bytes. Mask and clip
/// renderers clear every rectangle they paint after merging it, so the
/// buffer is still zero when it returns to the pool; a failed render may
/// stop midway and is cleared here.
fn with_coverage_scratch<R>(len: usize, visit: impl FnOnce(&mut [u8]) -> Option<R>) -> Option<R> {
    let mut scratch = COVERAGE_SCRATCH.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
    scratch.resize(len, 0);
    let result = visit(&mut scratch);
    if result.is_none() {
        scratch.fill(0);
    }
    debug_assert!(
        scratch.iter().all(|&byte| byte == 0),
        "coverage scratch must be clear"
    );
    COVERAGE_SCRATCH.with(|slot| *slot.borrow_mut() = scratch);
    result
}

thread_local! {
    static COVERAGE_SCRATCH: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    // Whole-surface mask coverage for the current render (render_mask_alpha).
    static MASK_CACHE: std::cell::RefCell<MaskCache> =
        std::cell::RefCell::new(MaskCache::default());
}

fn render_mask_alpha_in(
    mask: &usvg::Mask,
    parent_transform: Transform,
    width: usize,
    height: usize,
    region: &std::rc::Rc<LayerRegion>,
    scale: f32,
    device_origin: Point64,
    native: Option<(
        &crate::svg_precision::NativeConnectionMask,
        (usize, usize),
        [f64; 6],
    )>,
) -> Option<ZeroedLayer> {
    let mut alpha = ZeroedLayer::new(region.len);
    let native = native.filter(|(source, _, _)| {
        source.boxes.len() == mask.root().children().len()
            && mask
                .root()
                .children()
                .iter()
                .all(|child| matches!(child, usvg::Node::Path(_)))
    });
    // One zeroed coverage surface serves every mask path; each path clears
    // the rectangle it painted after merging it.
    with_coverage_scratch(alpha.len().checked_mul(4)?, |coverage| {
        render_mask_node(
            mask.root(),
            parent_transform,
            &mut alpha,
            coverage,
            width,
            height,
            region,
            scale,
            device_origin,
            native,
        )
    })?;
    Some(alpha)
}

fn render_mask_node(
    group: &usvg::Group,
    parent_transform: Transform,
    alpha: &mut [u8],
    coverage: &mut [u8],
    width: usize,
    height: usize,
    region: &std::rc::Rc<LayerRegion>,
    scale: f32,
    device_origin: Point64,
    native: Option<(
        &crate::svg_precision::NativeConnectionMask,
        (usize, usize),
        [f64; 6],
    )>,
) -> Option<()> {
    for (index, child) in group.children().iter().enumerate() {
        match child {
            usvg::Node::Group(group) => {
                render_mask_node(
                    group,
                    parent_transform.post_concat(group.abs_transform()),
                    alpha,
                    coverage,
                    width,
                    height,
                    region,
                    scale,
                    device_origin,
                    None,
                )?;
            }
            usvg::Node::Path(path) => {
                let Some(fill) = path.fill() else {
                    continue;
                };
                let Paint::Color(color) = fill.paint() else {
                    return None;
                };
                let device_transform = parent_transform
                    .post_concat(path.abs_transform())
                    .post_scale(scale, scale);
                if fill.rule() != FillRule::NonZero {
                    return None;
                }
                let mut rasterizer = Rasterizer::new(width, height, 0.375);
                let tolerance = flatten_tolerance(device_transform);
                let mut transform = native
                    .map(|(mask, (canvas_width, canvas_height), local)| {
                        affine_mul(
                            native_canvas_transform(mask.viewport, canvas_width, canvas_height),
                            Affine64 {
                                sx: local[0],
                                ky: local[1],
                                kx: local[2],
                                sy: local[3],
                                tx: local[4],
                                ty: local[5],
                            },
                        )
                    })
                    .unwrap_or_else(|| canonical_transform(device_transform));
                transform.tx -= device_origin.x;
                transform.ty -= device_origin.y;
                rasterizer.set_rounding_origin(Point64 {
                    x: -device_origin.x,
                    y: -device_origin.y,
                });
                let native_rect = native.and_then(|(mask, _, _)| mask.boxes.get(index));
                if let Some(&[x, y, w, h]) = native_rect {
                    // renderNode tests prepared primitive bounds before drawing
                    // mask children. A native connection cutout is a rectangle:
                    // its reference-rounded corners conservatively bound every
                    // submitted edge, without allocating or memoizing geometry
                    // for every other connection's effect reference origin.
                    let points = [
                        Point64 { x, y },
                        Point64 { x: x + w, y },
                        Point64 { x: x + w, y: y + h },
                        Point64 { x, y: y + h },
                    ];
                    let bounds = rasterizer.pixel_bounds(
                        points
                            .into_iter()
                            .map(|point| rasterizer.reference_point(map_point(point, transform))),
                    );
                    if bounds.is_none_or(|bounds| !region.intersects(bounds)) {
                        continue;
                    }
                }
                rasterizer.prepare_path(path, 4, transform, tolerance, |rasterizer| {
                    let paths = if let Some(rect) = native_rect {
                        vec![collect_scene_rect_centerline(
                            NativeRect {
                                box_values: *rect,
                                radii: [0.0; 2],
                            },
                            tolerance,
                        )]
                    } else {
                        collect_subpaths(path.data(), tolerance, false)
                    };
                    for (points, closed) in paths {
                        let mapped = map_centerline((points, closed), transform);
                        rasterizer.add_fill_polyline(&mapped.0, mapped.1);
                    }
                });
                // A fill covers nothing outside its edges' pixel bounds, and
                // zero coverage leaves the mask unchanged either way (max(a,
                // 0) = a; a * 255 / 255 = a), so merge only those bounds
                // within the region.
                let Some(bounds) = rasterizer.fill_pixel_bounds() else {
                    continue;
                };
                if !region.intersects(bounds) {
                    continue;
                }
                rasterizer.set_output_region(region.clone());
                let is_white = color.red >= 128 && color.green >= 128 && color.blue >= 128;
                // buildConnectionMask uses an opaque black primitive with
                // node opacity .75 for outside labels. Go rasterizes coverage
                // first, then compositeLayerOver rounds the opacity multiply;
                // the SVG transport folds that node opacity into its fill.
                let scene_hole = native.is_some() && !is_white && fill.opacity().get() < 1.0;
                let paint_alpha = if scene_hole {
                    255
                } else {
                    (255.0 * fill.opacity().get()).clamp(0.0, 255.0) as u8
                };
                let hole_opacity = (255.0 * fill.opacity().get()).round() as u32;
                rasterizer.paint(coverage, [255, 255, 255, paint_alpha]);
                region.for_each_index_in(bounds, |index| {
                    let coverage_pixel = &mut coverage[index * 4..index * 4 + 4];
                    let coverage_alpha = coverage_pixel[3];
                    coverage_pixel.fill(0);
                    if is_white {
                        alpha[index] = alpha[index].max(coverage_alpha);
                    } else if scene_hole {
                        let hole_alpha = (u32::from(coverage_alpha) * hole_opacity + 127) / 255;
                        alpha[index] =
                            ((u32::from(alpha[index]) * (255 - hole_alpha) + 127) / 255) as u8;
                    } else if native.is_some() {
                        // Opaque holes paint directly into the luminance
                        // mask with DrawRGBA's replicated 16-bit alpha. Its
                        // rounding also depends on earlier overlapping holes.
                        let mut pixel = [alpha[index]; 4];
                        blend_pixel(&mut pixel, [0, 0, 0, 255], coverage_alpha);
                        alpha[index] = pixel[0];
                    } else {
                        let inverse = 255_u16.saturating_sub(u16::from(coverage_alpha));
                        alpha[index] = (u16::from(alpha[index]) * inverse / 255) as u8;
                    }
                });
            }
            usvg::Node::Image(_) | usvg::Node::Text(_) => return None,
        }
    }
    Some(())
}

fn is_opaque_white_mask(mask: &usvg::Mask, group: &usvg::Group) -> bool {
    let mask_bounds = mask.root().abs_bounding_box();
    let group_bounds = group.abs_bounding_box();
    if mask_bounds.left() > group_bounds.left()
        || mask_bounds.top() > group_bounds.top()
        || mask_bounds.right() < group_bounds.right()
        || mask_bounds.bottom() < group_bounds.bottom()
    {
        return false;
    }
    let children = mask.root().children();
    !children.is_empty()
        && children.iter().all(|child| match child {
            usvg::Node::Path(path) => {
                path.stroke().is_none()
                    && path.fill().is_some_and(|fill| {
                        fill.opacity().get() == 1.0
                            && matches!(fill.paint(), Paint::Color(color) if color.red == 255 && color.green == 255 && color.blue == 255)
                    })
            }
            usvg::Node::Group(group) => is_opaque_white_group(group),
            usvg::Node::Image(_) | usvg::Node::Text(_) => false,
        })
}

fn is_trivial_clip(clip: &usvg::ClipPath, group: &usvg::Group) -> bool {
    // usvg represents an SVG viewport clip as an empty clip-path subroot; the
    // viewport itself is already reflected in the group's bounds.
    if clip.root().children().is_empty() {
        return true;
    }
    // Inverting a canvas-aligned bounding rectangle does not recover a
    // rotated object's local bounds. It enlarges them and makes an arrowhead
    // viewport appear to cut its own fully contained polygon, multiplying
    // antialiased border coverage a second time.
    let local_bounds = group.bounding_box();
    let covered = clip_children_cover_group(clip.root(), &local_bounds);
    covered
        && clip
            .clip_path()
            .is_none_or(|parent| is_trivial_clip(parent, group))
}

// A clip's ink bounds say nothing about coverage inside those bounds.
// Only an actual four-corner rectangle can use the viewport-cover shortcut;
// d2raster.renderEffectNode/applyClip retains curved and polygonal masks.
fn is_axis_aligned_clip_rectangle(path: &Path) -> bool {
    let mut points = Vec::new();
    let mut closed = false;
    for segment in path.segments() {
        if closed {
            return false;
        }
        match segment {
            PathSegment::MoveTo(point) if points.is_empty() => points.push(point),
            PathSegment::LineTo(point) if !points.is_empty() => points.push(point),
            PathSegment::Close => closed = true,
            _ => return false,
        }
    }
    if points.len() == 5 && points.first() == points.last() {
        points.pop();
    }
    if points.len() != 4 {
        return false;
    }
    let x0 = points[0].x;
    let y0 = points[0].y;
    for i in 0..4 {
        let a = points[i];
        let b = points[(i + 1) % 4];
        if (a.x == b.x) == (a.y == b.y) {
            return false;
        }
    }
    // Opposite corners of the nondegenerate orthogonal four-edge loop differ
    // on both axes; this also excludes folded or repeated-vertex polygons.
    points[2].x != x0 && points[2].y != y0
}

fn clip_children_cover_group(group: &usvg::Group, target: &resvg::tiny_skia::Rect) -> bool {
    !group.children().is_empty()
        && group.children().iter().any(|child| match child {
            usvg::Node::Path(path) => {
                if !is_axis_aligned_clip_rectangle(path.data()) {
                    return false;
                }
                let bounds = path.abs_bounding_box();
                // Clip subroots keep their own coordinate frame. Compare
                // extents as well as absolute bounds so marker/viewBox clips
                // are recognized after usvg has applied the parent transform.
                (bounds.left() <= target.left()
                    && bounds.top() <= target.top()
                    && bounds.right() >= target.right()
                    && bounds.bottom() >= target.bottom())
                    || (bounds.width() >= target.width() && bounds.height() >= target.height())
            }
            usvg::Node::Group(group) => clip_children_cover_group(group, target),
            usvg::Node::Image(_) | usvg::Node::Text(_) => false,
        })
}

fn is_opaque_white_group(group: &usvg::Group) -> bool {
    group.opacity().get() == 1.0
        && group.clip_path().is_none()
        && group.mask().is_none()
        && group.filters().is_empty()
        && group.blend_mode() == usvg::BlendMode::Normal
        && !group.children().is_empty()
        && group.children().iter().all(|child| match child {
            usvg::Node::Path(path) => {
                path.stroke().is_none()
                    && path.fill().is_some_and(|fill| {
                        fill.opacity().get() == 1.0
                            && matches!(fill.paint(), Paint::Color(color) if color.red == 255 && color.green == 255 && color.blue == 255)
                    })
            }
            usvg::Node::Group(group) => is_opaque_white_group(group),
            usvg::Node::Image(_) | usvg::Node::Text(_) => false,
        })
}

/// Rasterizes a simple solid-vector tree with D2's analytic coverage kernel.
/// Returns `None` when a scene feature requires the general resvg path.
#[cfg(test)]
pub fn render_simple(tree: &Tree, scale: u32) -> Option<(u32, u32, Vec<u8>)> {
    render_simple_with_sources(tree, scale, &[], &SvgImagePathSources::new())
}

#[cfg(test)]
pub fn render_simple_with_sources(
    tree: &Tree,
    scale: u32,
    sources: &[d2_rs_render::RasterPathSource],
    svg_image_paths: &SvgImagePathSources,
) -> Option<(u32, u32, Vec<u8>)> {
    render_simple_with_sources_and_background(tree, scale, sources, svg_image_paths, [255; 4])
}

pub fn render_simple_with_sources_and_background(
    tree: &Tree,
    scale: u32,
    sources: &[d2_rs_render::RasterPathSource],
    svg_image_paths: &SvgImagePathSources,
    background: [u8; 4],
) -> Option<(u32, u32, Vec<u8>)> {
    render_simple_with_sources_and_background_and_dash_parameters(
        tree,
        scale,
        sources,
        svg_image_paths,
        background,
        &BTreeMap::new(),
    )
}

#[derive(Clone, Copy)]
struct NativePathContext {
    transform: Affine64,
    parsed_content_transform: Transform,
}

fn synthetic_italic_group_context(
    group: &usvg::Group,
    parent: NativePathContext,
    sources: &SvgImagePathSources,
) -> Option<NativePathContext> {
    // usvg moves a text element's transform to an anonymous single-text
    // wrapper. Only the native private run with matching pivot operands
    // qualifies; an unrelated SVG transform cannot inherit this transport.
    let [usvg::Node::Text(text)] = group.children() else {
        return None;
    };
    sources.exact_text_origin_by_id(text.id())?;
    let (x, y) = synthetic_italic_pivot(text)?;
    let local = synthetic_italic_local_f32(x, y);
    if group.transform() != local
        || group.abs_transform() != parent.parsed_content_transform.pre_concat(local)
        || text.abs_transform() != group.abs_transform()
    {
        return None;
    }
    Some(NativePathContext {
        parsed_content_transform: group.abs_transform(),
        transform: affine_mul(parent.transform, synthetic_italic_local_f64(x, y)),
    })
}

// d2raster maps the scene viewBox directly into the integer frame.
fn native_canvas_transform(
    viewport: crate::svg_precision::NativeCanvasViewport,
    width: usize,
    height: usize,
) -> Affine64 {
    let [x, y, view_width, view_height] = viewport.view_box;
    let uniform = (width as f64 / view_width).min(height as f64 / view_height);
    let offset_x = if viewport.center {
        (width as f64 - view_width * uniform) / 2.0
    } else {
        0.0
    };
    let offset_y = if viewport.center {
        (height as f64 - view_height * uniform) / 2.0
    } else {
        0.0
    };
    Affine64 {
        sx: uniform,
        kx: 0.0,
        ky: 0.0,
        sy: uniform,
        tx: offset_x - x * uniform,
        ty: offset_y - y * uniform,
    }
}

fn collect_native_scene_transforms(
    group: &usvg::Group,
    inherited: Option<NativePathContext>,
    width: usize,
    height: usize,
    sources: &SvgImagePathSources,
    output: &mut HashMap<*const usvg::Path, Affine64>,
    text_output: &mut HashMap<*const usvg::Text, Affine64>,
    shadow_output: &mut HashMap<*const usvg::Group, Affine64>,
) {
    let context = if let Some(viewport) = sources.native_canvas_geometry(group.id()) {
        let [x, y, _, _] = viewport.view_box;
        Some(NativePathContext {
            parsed_content_transform: group.abs_transform().pre_translate(-x as f32, -y as f32),
            transform: native_canvas_transform(viewport, width, height),
        })
    } else if let (Some(parent), Some([x, y, _, _])) =
        (inherited, sources.native_markdown_viewport(group.id()))
    {
        #[cfg(feature = "diagnostic-traces")]
        if raster_trace_pixel().is_some() {
            eprintln!(
                "d2-rs-md-viewport-context id={:?} abs={:?} parent_parsed={:?} xy=({x},{y})",
                group.id(),
                group.abs_transform(),
                parent.parsed_content_transform
            );
        }
        // buildMarkdownLabel retains its placement as a separate scene
        // translation. usvg's clip group precedes the translated content.
        (group.abs_transform() == parent.parsed_content_transform).then(|| NativePathContext {
            parsed_content_transform: group.abs_transform().pre_translate(x as f32, y as f32),
            transform: affine_mul(
                parent.transform,
                Affine64 {
                    sx: 1.0,
                    kx: 0.0,
                    ky: 0.0,
                    sy: 1.0,
                    tx: x,
                    ty: y,
                },
            ),
        })
    } else if let (Some(parent), Some(local)) =
        (inherited, sources.native_group_transform(group.id()))
    {
        Some(NativePathContext {
            parsed_content_transform: group.abs_transform(),
            transform: affine_mul(
                parent.transform,
                Affine64 {
                    sx: local[0],
                    ky: local[1],
                    kx: local[2],
                    sy: local[3],
                    tx: local[4],
                    ty: local[5],
                },
            ),
        })
    } else if let Some(context) =
        inherited.and_then(|parent| synthetic_italic_group_context(group, parent, sources))
    {
        Some(context)
    } else if inherited
        .is_some_and(|context| group.abs_transform() == context.parsed_content_transform)
        || group.transform().is_identity()
    {
        inherited
    } else {
        // Local authored transforms need their own source operands; never
        // infer them by undoing a rounded absolute matrix.
        None
    }
    .filter(|context| {
        let transform = context.transform;
        [
            transform.sx,
            transform.kx,
            transform.ky,
            transform.sy,
            transform.tx,
            transform.ty,
        ]
        .iter()
        .all(|value| value.is_finite())
            && context.parsed_content_transform.is_finite()
    });
    #[cfg(feature = "diagnostic-traces")]
    if raster_trace_pixel().is_some() && inherited.is_some() && context.is_none() {
        eprintln!(
            "d2-rs-native-context-dropped id={:?} transform={:?} abs={:?} parent_parsed={:?}",
            group.id(),
            group.transform(),
            group.abs_transform(),
            inherited.map(|c| c.parsed_content_transform)
        );
    }
    if let Some(context) = context {
        if sources.native_markdown_viewport(group.id()).is_some()
            || ((sources.is_native_shape_shadow(group.id())
                || sources.is_native_legend_shadow(group.id()))
                && group.abs_transform() == context.parsed_content_transform)
        {
            shadow_output.insert(group as *const usvg::Group, context.transform);
        }
    }
    for child in group.children() {
        match child {
            usvg::Node::Group(child) => collect_native_scene_transforms(
                child,
                context,
                width,
                height,
                sources,
                output,
                text_output,
                shadow_output,
            ),
            usvg::Node::Path(path) => {
                if let Some(context) = context
                    .filter(|context| path.abs_transform() == context.parsed_content_transform)
                {
                    // Pointer identity binds duplicate/empty SVG path IDs to
                    // their actual retained node. Keys are never dereferenced.
                    output.insert(path.as_ref() as *const usvg::Path, context.transform);
                }
            }
            usvg::Node::Text(text) => {
                if let Some(context) = context {
                    let exact = if text.abs_transform() == context.parsed_content_transform {
                        Some(context.transform)
                    } else {
                        // Renderer-authored synthetic italic has source pivot
                        // operands. Bind those to the actual resolved matrix;
                        // unrelated authored text transforms stay unqualified.
                        synthetic_italic_pivot(text).and_then(|(x, y)| {
                            (text.abs_transform()
                                == context
                                    .parsed_content_transform
                                    .pre_concat(synthetic_italic_local_f32(x, y)))
                            .then(|| {
                                affine_mul(context.transform, synthetic_italic_local_f64(x, y))
                            })
                        })
                    };
                    if let Some(exact) = exact {
                        text_output.insert(text.as_ref() as *const usvg::Text, exact);
                    }
                }
            }
            _ => {}
        }
    }
}

// Children paint sequentially. Only ancestor effect buffers coexist with
// a descendant's peak layer/alpha/coverage scratch. Filters budget their
// prepared input crop separately in draw_native_sharp_shadow.
fn effect_layer_units(group: &usvg::Group, include_self: bool) -> usize {
    let own = if !include_self || !group.filters().is_empty() {
        0
    } else if group.mask().is_some() || group.clip_path().is_some() {
        3
    } else if group.opacity().get() != 1.0 || group.blend_mode() != usvg::BlendMode::Normal {
        1
    } else {
        0
    };
    own + group
        .children()
        .iter()
        .filter_map(|node| match node {
            usvg::Node::Group(child) => Some(effect_layer_units(child, true)),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

pub fn render_simple_with_sources_and_background_and_dash_parameters(
    tree: &Tree,
    scale: u32,
    sources: &[d2_rs_render::RasterPathSource],
    svg_image_paths: &SvgImagePathSources,
    background: [u8; 4],
    exact_dash_parameters: &BTreeMap<String, (f64, f64, f64)>,
) -> Option<(u32, u32, Vec<u8>)> {
    render_simple_with_band_height(
        tree,
        scale,
        sources,
        svg_image_paths,
        background,
        exact_dash_parameters,
        None,
        None,
    )
}

/// Receives a frame band by band, top to bottom: frame width, frame height,
/// and the band's full-width RGBA rows.
pub(crate) type BandSink<'a> = &'a mut dyn FnMut(usize, usize, &mut [u8]);

/// Render like `render_simple_with_sources_and_background_and_dash_parameters`,
/// but hand the frame to `sink` in consecutive full-width row bands instead
/// of returning it, so a large frame is never resident at once. Bands are
/// final when emitted; None means the native renderer declined the tree
/// (possibly after some bands were emitted). A paged frame that is cheaper to
/// render whole is returned instead, with nothing emitted.
pub(crate) fn render_simple_bands(
    tree: &Tree,
    scale: u32,
    sources: &[d2_rs_render::RasterPathSource],
    svg_image_paths: &SvgImagePathSources,
    background: [u8; 4],
    exact_dash_parameters: &BTreeMap<String, (f64, f64, f64)>,
    sink: BandSink<'_>,
) -> Option<(u32, u32, Vec<u8>)> {
    render_simple_with_band_height(
        tree,
        scale,
        sources,
        svg_image_paths,
        background,
        exact_dash_parameters,
        None,
        Some(sink),
    )
}

/// Frame bytes rendered per band pass when a frame is streamed.
const STREAM_BAND_BYTES: usize = 4 << 20;

fn render_simple_with_band_height(
    tree: &Tree,
    scale: u32,
    sources: &[d2_rs_render::RasterPathSource],
    svg_image_paths: &SvgImagePathSources,
    background: [u8; 4],
    exact_dash_parameters: &BTreeMap<String, (f64, f64, f64)>,
    requested_band_height: Option<usize>,
    mut sink: Option<BandSink<'_>>,
) -> Option<(u32, u32, Vec<u8>)> {
    // A new tree may reuse a previous tree's address.
    reset_scene_fallback_buckets();
    struct MaskCacheScope;
    impl Drop for MaskCacheScope {
        fn drop(&mut self) {
            MASK_CACHE.with(|cache| *cache.borrow_mut() = MaskCache::default());
            COVERAGE_SCRATCH.with(|scratch| *scratch.borrow_mut() = Vec::new());
        }
    }
    MASK_CACHE.with(|cache| *cache.borrow_mut() = MaskCache::default());
    let _mask_cache = MaskCacheScope;
    let _text_memo = TextMemoScope::new();
    let mut raster_sources = RasterSourceMap::from_paths(sources, exact_dash_parameters);
    raster_sources.native_rects = svg_image_paths.native_rects().clone();
    raster_sources.gradient_stops = svg_image_paths.gradient_stops().clone();
    if !raster_sources.gradient_stops.is_empty() {
        let mut paints = HashMap::new();
        native_gradient::collect(tree.root(), &raster_sources, svg_image_paths, &mut paints);
        raster_sources.native_gradient_paints = paints;
    }
    let [logical_width, logical_height] = svg_image_paths.logical_size(tree.size());
    let width = usize::try_from(crate::svg_precision::scaled_dimension(
        logical_width,
        scale,
    )?)
    .ok()?;
    let height = usize::try_from(crate::svg_precision::scaled_dimension(
        logical_height,
        scale,
    )?)
    .ok()?;
    collect_native_scene_transforms(
        tree.root(),
        None,
        width,
        height,
        svg_image_paths,
        &mut raster_sources.native_path_transforms,
        &mut raster_sources.native_text_transforms,
        &mut raster_sources.native_shadow_transforms,
    );
    pattern::prepare(tree, scale, &mut raster_sources, svg_image_paths)?;
    collect_paint_reference_contexts(
        tree.root(),
        PaintReferenceContext {
            paint_bounds: Some(TextPaintBounds {
                left: 0,
                top: 0,
                right: i64::try_from(width).ok()?,
                bottom: i64::try_from(height).ok()?,
            }),
        },
        &mut raster_sources.paint_reference_contexts,
        svg_image_paths,
        tree,
        scale as f32,
    );
    let frame_bytes = width.checked_mul(height)?.checked_mul(4)?;
    let row_bytes = width.checked_mul(4)?;
    const MAX_EFFECT_FRAME_BYTES: usize = 256 * 1024 * 1024;
    // Keep the previously qualified full native path for trees without
    // sharp shadows. Large shadow trees require bounded effects; image,
    // color-font and appendix transport through bands remains unqualified.
    // A streamed frame bounds its effect layers by band passes instead.
    if requested_band_height.is_some()
        || (sink.is_none()
            && frame_bytes > MAX_EFFECT_FRAME_BYTES
            && !raster_sources.native_shadow_transforms.is_empty())
    {
        // Original d2cli/raster_png_output.go uses 256-row bands. Each mask,
        // clip and opacity layer now scales with this destination, while text
        // and path submission retain their complete-frame rounding reference.
        raster_sources.banded = true;
        raster_sources.canvas_height = height;
        let layer_units = effect_layer_units(tree.root(), true).checked_add(1)?;
        let max_rows = MAX_EFFECT_FRAME_BYTES.checked_div(row_bytes.checked_mul(layer_units)?)?;
        let band_height = requested_band_height
            .unwrap_or(256)
            .min(height)
            .min(max_rows);
        if band_height == 0 {
            return None;
        }
        raster_sources.band_scratch_bytes = row_bytes
            .checked_mul(band_height)?
            .checked_mul(layer_units)?;
        // A streamed frame is handed over band by band; otherwise collect it.
        let mut pixels = if sink.is_some() {
            Vec::new()
        } else {
            vec![0_u8; frame_bytes]
        };
        let mut band = vec![0_u8; row_bytes.checked_mul(band_height)?];
        for top in (0..height).step_by(band_height) {
            let rows = band_height.min(height - top);
            let bytes = width.checked_mul(rows)?.checked_mul(4)?;
            for pixel in band[..bytes].chunks_exact_mut(4) {
                pixel.copy_from_slice(&background);
            }
            raster_sources.device_origin = Point64 {
                x: 0.0,
                y: top as f64,
            };
            if !draw_group(
                tree,
                tree.root(),
                &mut band[..bytes],
                width,
                rows,
                scale as f32,
                1.0,
                false,
                &raster_sources,
                svg_image_paths,
            ) {
                return None;
            }
            match sink.as_mut() {
                Some(sink) => sink(width, height, &mut band[..bytes]),
                None => pixels[top * row_bytes..(top + rows) * row_bytes]
                    .copy_from_slice(&band[..bytes]),
            }
        }
        return Some((width as u32, height as u32, pixels));
    }
    if GO_PNG_BANDS.with(std::cell::Cell::get) {
        GO_BAND_FRAME.with(|frame| {
            *frame.borrow_mut() = Some(GoBandFrame {
                width,
                height,
                layers: Vec::new(),
            });
        });
    }
    struct FrameScope;
    impl Drop for FrameScope {
        fn drop(&mut self) {
            GO_BAND_FRAME.with(|frame| *frame.borrow_mut() = None);
            GO_BAND_LEAF_CACHE.with(|cache| cache.borrow_mut().clear());
            GO_BAND_LAYER_CACHE.with(|cache| cache.borrow_mut().clear());
            FRAME_BAND.with(|band| band.set(None));
        }
    }
    let _frame_scope = FrameScope;
    let draw_frame = |pixels: &mut [u8]| {
        draw_group(
            tree,
            tree.root(),
            pixels,
            width,
            height,
            scale as f32,
            1.0,
            false,
            &raster_sources,
            svg_image_paths,
        )
    };
    let fill_background = |rows: &mut [u8]| {
        for pixel in rows.chunks_exact_mut(4) {
            pixel.copy_from_slice(&background);
        }
    };
    let band_rows = (STREAM_BAND_BYTES / row_bytes.max(1)).max(1);
    // A PNG frame always streams. A paged frame streams only when its effect
    // layers could hold several frames' worth of memory at once and no sharp
    // shadow would be blurred again for every band; otherwise one whole-frame
    // pass is cheaper and is returned to the caller as is.
    let png_bands = GO_PNG_BANDS.with(std::cell::Cell::get);
    // A legend is one prepared scene subtree in d2raster. Replaying the
    // serialized subtree across Rust's output-stream chunks can reuse
    // reference state from a preceding chunk and suppress its later children.
    // Render bounded legend frames once, then stream the completed frame to
    // the encoder; large frames retain the bounded band path.
    let whole_legend = png_bands
        && frame_bytes <= MAX_EFFECT_FRAME_BYTES
        && svg_image_paths.has_native_legend_panel();
    let stream = (png_bands && !whole_legend)
        || (!png_bands
            && raster_sources.native_shadow_transforms.is_empty()
            && frame_bytes.saturating_mul(effect_layer_units(tree.root(), true))
                >= PAGED_STREAM_LAYER_BYTES);
    TEXT_MEMO.with(|memo| {
        if let Some(memo) = memo.borrow_mut().as_mut() {
            // A whole-frame paged render submits each path once. Retain its
            // geometry only when later destination passes will reuse it.
            memo.raster_surface = stream.then_some((width, height));
        }
    });
    if let Some(sink) = sink.as_mut()
        && stream
        && height > band_rows
    {
        // Stream a large frame: every band pass draws the complete scene into
        // a full-frame buffer whose geometry, references and effect windows
        // are exactly those of a whole-frame render, but writes only that
        // band's rows (see FRAME_BAND). The finished rows are handed to the
        // sink and their memory released before the next band.
        let mut frame = ZeroedLayer::new(frame_bytes);
        for top in (0..height).step_by(band_rows) {
            let bottom = (top + band_rows).min(height);
            let rows = top * row_bytes..bottom * row_bytes;
            fill_background(&mut frame[rows.clone()]);
            FRAME_BAND.with(|band| band.set(Some((width, height, top, bottom))));
            let drawn = draw_frame(&mut frame);
            FRAME_BAND.with(|band| band.set(None));
            if !drawn {
                return None;
            }
            TEXT_MEMO.with(|memo| {
                if let Some(memo) = memo.borrow_mut().as_mut() {
                    memo.effect_layer = None;
                    // A completed destination no longer needs its expanded
                    // scan edges. Keep only primitives that reach later rows;
                    // compact source paths remain available for reconstruction.
                    memo.rasters.retain(|_, raster| {
                        raster.bounds.is_some_and(|(_, _, _, end)| end > bottom)
                    });
                }
            });
            sink(width, height, &mut frame[rows.clone()]);
            frame.discard(rows);
        }
        return Some((width as u32, height as u32, Vec::new()));
    }
    if let Some(sink) = sink.as_mut()
        && !png_bands
        && background == [0; 4]
    {
        // Paged PNGs can consume a whole frame directly, including scenes
        // whose sharp shadows make repeated band passes too expensive.
        // Anonymous pages start transparent; leave the unpainted canvas
        // untouched instead of filling and retaining a full-frame Vec.
        let mut pixels = ZeroedLayer::new(frame_bytes);
        if !draw_frame(&mut pixels) {
            return None;
        }
        sink(width, height, &mut pixels);
        return Some((width as u32, height as u32, Vec::new()));
    }
    let mut pixels = vec![0_u8; frame_bytes];
    fill_background(&mut pixels);
    if !draw_frame(&mut pixels) {
        return None;
    }
    if let Some(sink) = sink.as_mut()
        && png_bands
    {
        sink(width, height, &mut pixels);
        return Some((width as u32, height as u32, Vec::new()));
    }
    Some((width as u32, height as u32, pixels))
}

/// Effect-layer bytes (frame bytes times nested layer units) above which a
/// paged frame renders band by band.
const PAGED_STREAM_LAYER_BYTES: usize = 64 << 20;

// Prepare pattern resources in dependency order. Source preflight descends
// into each pattern root before publishing its tile (d2raster/pattern.go).
#[cfg(test)]
mod tests {
    /// A streamed frame is the whole-frame render, band by band: masks,
    /// opacity layers, strokes, curves and text crossing band boundaries.
    /// PNG frames stream under Go's band emulation, paged frames without it.
    #[test]
    fn streamed_bands_match_whole_frame_render() {
        // Paged sharp-shadow frames deliberately render in one pass; PNG
        // sharp-shadow frames stream and need regional filter inputs.
        for (go_bands, sharp_shadow) in [(true, false), (false, false), (true, true)] {
            streamed_bands_match_whole_frame_render_with(go_bands, sharp_shadow);
        }
    }

    fn streamed_bands_match_whole_frame_render_with(go_bands: bool, sharp_shadow: bool) {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="6000" viewBox="0 0 320 6000" preserveAspectRatio="xMinYMin meet">
          <defs><mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="320" height="6000">
            <rect width="320" height="6000" fill="white"/>
            <rect x="100" y="860" width="80" height="40" fill="black"/>
          </mask></defs>
          <rect x="10" y="10" width="300" height="3900" rx="20" fill="#eef1f8" stroke="#335" stroke-width="3"/>
          <path d="M 40 40 C 300 900 -100 2500 250 3900" stroke="#0D32B2" stroke-width="2" fill="none" mask="url(#m)"/>
          <g opacity="0.5"><circle cx="160" cy="1740" r="120" fill="#c33"/><rect x="60" y="1700" width="200" height="90" fill="#3c3"/></g>
          <text x="40" y="880" font-family="Source Sans Pro" font-size="28" fill="#111">Band edge text</text>
          <text x="40" y="2625" font-family="Source Sans Pro" font-size="40" fill="#123">Another line</text>
        </svg>"##;
        // The tall-shadow-box stress witness needs source rows just above
        // each band as well as the unshifted shape. Fractional viewport
        // scaling also exercises bilinear sampling across that boundary.
        let svg = if sharp_shadow {
            svg.replacen("width=\"320\" height=\"6000\"", "width=\"400\" height=\"7500\"", 1).replace(
                "<defs>",
                r##"<defs><filter id="shadow" x="-20%" y="-20%" width="160%" height="160%"><feFlood flood-color="#3d4574" flood-opacity="0.4" result="shadow-color"/><feComposite in="shadow-color" in2="SourceAlpha" operator="in"/><feOffset dx="3" dy="5"/><feBlend in="SourceGraphic" mode="normal"/></filter>"##,
            ).replace(
                "<rect x=\"10\"",
                r##"<svg id="d2-rs-native-canvas-viewport-shadow" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="320" height="6000" viewBox="0 0 320 6000"><g id="d2-rs-native-shape-shadow-control" data-d2-rs-shape-shadow="sharp" filter="url(#shadow)"><rect id="control-native-raster-shape-0" x="-2.25" y="-3.125" width="220" height="4700" rx="7.75" fill="#eef1f8" stroke="#335"/><g opacity="0.6"><rect id="control-native-raster-shape-1" x="15" y="100" width="120" height="4200" fill="#f69d3c"/></g></g></svg><rect x="10""##,
            )
        } else {
            svg.to_owned()
        };
        let (svg, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
        if sharp_shadow {
            assert!(sources.is_native_shape_shadow("d2-rs-native-shape-shadow-control"));
        }
        let mut options = usvg::Options::default();
        super::super::load_bundled_fonts(&mut options);
        let tree = usvg::Tree::from_str(&svg, &options).unwrap();
        let _bands = go_bands.then(super::GoPngBandsScope::new);
        let render = |sink: Option<super::BandSink<'_>>| {
            super::render_simple_with_band_height(
                &tree,
                2,
                &[],
                &sources,
                [255; 4],
                &std::collections::BTreeMap::new(),
                None,
                sink,
            )
            .unwrap()
        };
        let (width, height, whole) = render(None);
        assert!(whole.len() > 2 * super::STREAM_BAND_BYTES);
        let mut streamed = Vec::new();
        let mut bands = 0;
        let mut sink = |frame_width: usize, frame_height: usize, band: &mut [u8]| {
            assert_eq!(
                (frame_width, frame_height),
                (width as usize, height as usize)
            );
            bands += 1;
            streamed.extend_from_slice(band);
        };
        let (_, _, empty) = render(Some(&mut sink));
        assert!(empty.is_empty() && bands > 2);
        assert!(
            streamed == whole,
            "streamed bands differ from the whole frame (Go bands: {go_bands}, sharp shadow: {sharp_shadow})"
        );
    }

    #[test]
    fn png_legend_scene_is_completed_before_streaming() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="667" height="818" viewBox="0 0 667 818" preserveAspectRatio="xMinYMin meet">
          <defs><filter id="ls" x="-100%" y="-100%" width="300%" height="300%" color-interpolation-filters="sRGB"><feDropShadow dx="0" dy="2" stdDeviation="3" flood-color="#000000" flood-opacity="0.10196078431372549"/></filter></defs>
          <svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="667" height="818" viewBox="-101 -101 667 818">
            <rect x="-101" y="-101" width="667" height="818" fill="white"/>
            <g id="d2-rs-native-legend-shadow-card" data-d2-rs-legend-shadow="scene" filter="url(#ls)"><rect id="d2-rs-native-legend-shadow-rect" x="326" y="350" width="190" height="267" rx="4" fill="#F7F7FA" stroke="#DEE1EB" stroke-width="1"/></g>
            <g><rect id="d2-rs-native-legend-panel" x="326" y="350" width="190" height="267" rx="4" fill="#ffffff" stroke="#DEE1EB" stroke-width="1"/><text id="d2-rs-native-legend-title" x="346" y="384" font-family="Source Sans Pro" font-size="16" font-weight="bold">Legend</text><rect x="346" y="404" width="24" height="24" fill="#F7F8FE" stroke="#0D32B2"/><text x="390" y="421" font-family="Source Sans Pro" font-size="14">Microservice</text></g>
          </svg>
        </svg>"##;
        let (svg, sources) = crate::svg_precision::tag_embedded_svg_images(svg);
        assert!(sources.has_native_legend_panel());
        let mut options = usvg::Options::default();
        super::super::load_bundled_fonts(&mut options);
        let tree = usvg::Tree::from_str(&svg, &options).unwrap();
        let render = |sink: Option<super::BandSink<'_>>| {
            super::render_simple_with_band_height(
                &tree,
                2,
                &[],
                &sources,
                [255; 4],
                &std::collections::BTreeMap::new(),
                None,
                sink,
            )
            .unwrap()
        };
        let (_, _, whole) = render(None);
        let _bands = super::GoPngBandsScope::new();
        let mut streamed = Vec::new();
        let mut calls = 0;
        let mut sink = |_: usize, _: usize, band: &mut [u8]| {
            calls += 1;
            streamed.extend_from_slice(band);
        };
        let (_, _, empty) = render(Some(&mut sink));
        assert!(empty.is_empty());
        assert_eq!(calls, 1, "bounded legend scene is completed in one pass");
        assert_eq!(streamed, whole);
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn markdown_clip_reference_matches_original_prepared_scene() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="262" height="162" viewBox="0 0 262 162"><svg id="d2-rs-native-canvas-viewport-main" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="262" height="162" viewBox="-21 -21 262 162"><svg id="d2-rs-native-md-viewport-control" data-d2-rs-markdown-reference="ordinary" x="91" y="48" width="38.000" height="24.000" viewBox="0 0 38.000 24.000" overflow="hidden"><g><rect id="d2-rs-native-md-rect-control" x="0" y="2.2799999999999994" width="35.3440625" height="22.44" rx="6" fill="#eef1f8"/><text id="native" x="5.44" y="18" data-d2-rs-text-x-bits="4015c28f5c28f5c3" data-d2-rs-text-y-bits="4032000000000000" font-family="Source Code Pro" font-size="13.6" fill="#0a0f25" text-decoration="underline">Agm</text></g></svg></svg></svg>"##;
        let (svg, sources) = super::super::svg_precision::tag_embedded_svg_images(svg);
        let mut options = usvg::Options::default();
        super::super::load_bundled_fonts(&mut options);
        let tree = usvg::Tree::from_str(&svg, &options).unwrap();
        let mut contexts = std::collections::HashMap::new();
        super::collect_paint_reference_contexts(
            tree.root(),
            super::PaintReferenceContext {
                paint_bounds: Some(super::TextPaintBounds {
                    left: 0,
                    top: 0,
                    right: 524,
                    bottom: 324,
                }),
            },
            &mut contexts,
            &sources,
            &tree,
            2.0,
        );
        let expected: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-reference-source-v090.json"
        ))
        .unwrap();
        let source = &expected[2]["reference"];
        assert_eq!(
            contexts["native"].paint_bounds,
            Some(super::TextPaintBounds {
                left: source["Min"]["X"].as_i64().unwrap(),
                top: source["Min"]["Y"].as_i64().unwrap(),
                right: source["Max"]["X"].as_i64().unwrap(),
                bottom: source["Max"]["Y"].as_i64().unwrap()
            })
        );
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn markdown_reference_matrix_matches_original_prepared_scene() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-reference-matrix-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 96);
        let mut options = usvg::Options::default();
        super::super::load_bundled_fonts(&mut options);
        for (index, row) in rows.as_array().unwrap().iter().enumerate() {
            let (svg, sources) =
                super::super::svg_precision::tag_embedded_svg_images(row["svg"].as_str().unwrap());
            let tree = usvg::Tree::from_str(&svg, &options).unwrap();
            let scale = row["scale"].as_f64().unwrap() as f32;
            let mut contexts = std::collections::HashMap::new();
            super::collect_paint_reference_contexts(
                tree.root(),
                super::PaintReferenceContext {
                    paint_bounds: Some(super::TextPaintBounds {
                        left: 0,
                        top: 0,
                        right: (262.0 * scale) as i64,
                        bottom: (162.0 * scale) as i64,
                    }),
                },
                &mut contexts,
                &sources,
                &tree,
                scale,
            );
            assert!(!contexts.is_empty(), "case {index}");
            let reference = &row["reference"];
            let expected = super::TextPaintBounds {
                left: reference["Min"]["X"].as_i64().unwrap(),
                top: reference["Min"]["Y"].as_i64().unwrap(),
                right: reference["Max"]["X"].as_i64().unwrap(),
                bottom: reference["Max"]["Y"].as_i64().unwrap(),
            };
            for (id, context) in contexts {
                assert_eq!(
                    context.paint_bounds,
                    Some(expected),
                    "case {index}, text {id}"
                );
            }
        }
    }
    use super::{
        Point64, Rasterizer, clean_stroke_points, direct_svg_group_fits_viewport,
        exact_dash_parameters, exact_dash_parameters_from_id, group_reference_origin,
        path_scene_transform, render_simple,
    };
    use resvg::{
        tiny_skia::{Point, Transform},
        usvg,
    };

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn markdown_italic_ancestry_matches_complete_original_go_frames() {
        use base64::Engine;
        let cases: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-italic-ancestry-source-v090.json"
        ))
        .unwrap();
        assert_eq!(cases.as_array().unwrap().len(), 16);
        for (index, case) in cases.as_array().unwrap().iter().enumerate() {
            let prepared = super::super::emulate_go_synthetic_italic(case["svg"].as_str().unwrap());
            let (prepared, sources) =
                super::super::svg_precision::tag_embedded_svg_images(&prepared);
            let mut options = usvg::Options::default();
            super::super::load_bundled_fonts(&mut options);
            let tree = usvg::Tree::from_str(&prepared, &options).unwrap();
            let scale = case["scale"].as_u64().unwrap() as u32;
            let mut paths = std::collections::HashMap::new();
            let mut texts = std::collections::HashMap::new();
            let mut shadows = std::collections::HashMap::new();
            super::collect_native_scene_transforms(
                tree.root(),
                None,
                (100 * scale) as usize,
                (64 * scale) as usize,
                &sources,
                &mut paths,
                &mut texts,
                &mut shadows,
            );
            assert_eq!(
                texts.len(),
                1,
                "private synthetic wrapper retains its source transform, case {index}"
            );
            let exact = texts.values().next().unwrap();
            let expected_matrix = &case["matrix"];
            for (field, actual) in [
                ("A", exact.sx),
                ("B", exact.ky),
                ("C", exact.kx),
                ("D", exact.sy),
                ("E", exact.tx),
                ("F", exact.ty),
            ] {
                assert_eq!(
                    actual.to_bits(),
                    expected_matrix[field].as_f64().unwrap().to_bits(),
                    "source matrix {field}, case {index}"
                );
            }
            let unqualified =
                prepared.replace(" data-d2-rs-text-x-bits=", " data-unqualified-text-x-bits=");
            let (unqualified, unqualified_sources) =
                super::super::svg_precision::tag_embedded_svg_images(&unqualified);
            let unqualified_tree = usvg::Tree::from_str(&unqualified, &options).unwrap();
            let mut unqualified_texts = std::collections::HashMap::new();
            super::collect_native_scene_transforms(
                unqualified_tree.root(),
                None,
                (100 * scale) as usize,
                (64 * scale) as usize,
                &unqualified_sources,
                &mut paths,
                &mut unqualified_texts,
                &mut shadows,
            );
            assert!(
                unqualified_texts.is_empty(),
                "an unqualified wrapper cannot acquire source precision"
            );
            let (width, height, pixels) =
                super::render_simple_with_sources(&tree, scale, &[], &sources)
                    .expect("native synthetic italic Markdown");
            assert_eq!((width, height), (100 * scale, 64 * scale));
            let expected = base64::engine::general_purpose::STANDARD
                .decode(case["pixels"].as_str().unwrap())
                .unwrap();
            assert_eq!(pixels.len(), expected.len());
            let differences = pixels.iter().zip(&expected).filter(|(a, b)| a != b).count();
            assert_eq!(differences, 0, "complete Source frame {index}");
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn transformed_text_strokes_match_complete_original_go_frames() {
        use base64::Engine;
        let cases: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/text-stroke-source-v090.json"
        ))
        .unwrap();
        for (index, case) in cases.as_array().unwrap().iter().enumerate() {
            let m = case["matrix"].as_array().unwrap();
            let input = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="64"><g transform="matrix({} {} {} {} {} {})"><text class="md-text text-mono" x="5.44" y="32" font-family="Source Code Pro" font-size="{}" fill="#0a0f25" stroke="#0a0f25" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round" stroke-miterlimit="4" data-d2-rs-text-x-bits="{:016x}" data-d2-rs-text-y-bits="{:016x}">Agm</text></g></svg>"##,
                m[0],
                m[1],
                m[2],
                m[3],
                m[4],
                m[5],
                case["size"],
                case["width"],
                5.44_f64.to_bits(),
                32_f64.to_bits(),
            );
            let prepared = super::super::emulate_go_synthetic_italic(&input);
            let (prepared, sources) =
                super::super::svg_precision::tag_embedded_svg_images(&prepared);
            let mut options = usvg::Options::default();
            super::super::load_bundled_fonts(&mut options);
            let tree = usvg::Tree::from_str(&prepared, &options).unwrap();
            fn first_text(group: &usvg::Group) -> Option<&usvg::Text> {
                group.children().iter().find_map(|child| match child {
                    usvg::Node::Text(text) => Some(text.as_ref()),
                    usvg::Node::Group(group) => first_text(group),
                    _ => None,
                })
            }
            let text = first_text(tree.root()).unwrap();
            let parsed_width = text.layouted()[0].stroke.as_ref().unwrap().width().get();
            assert_eq!(
                sources.text_stroke_width(text.id(), parsed_width).to_bits(),
                case["width"].as_f64().unwrap().to_bits(),
                "actual resolved Source paint retains f64 width",
            );
            assert_eq!(
                sources.text_stroke_width(text.id(), 2.0),
                2.0,
                "a different CSS-resolved paint cannot acquire the source operand"
            );
            let scale = case["scale"].as_u64().unwrap() as u32;
            let (width, height, pixels) =
                super::render_simple_with_sources(&tree, scale, &[], &sources)
                    .expect("native text stroke path");
            assert_eq!((width, height), (100 * scale, 64 * scale));
            let expected = base64::engine::general_purpose::STANDARD
                .decode(case["pixels"].as_str().unwrap())
                .unwrap();
            assert_eq!(pixels.len(), expected.len());
            let differences = pixels.iter().zip(&expected).filter(|(a, b)| a != b).count();
            assert_eq!(differences, 0, "complete Source frame {index}");
        }
    }

    #[test]
    fn synthetic_italic_preserves_nonzero_text_baseline_origins() {
        // d2scenebuild.buildMarkdownPrimitive anchors its synthetic skew at
        // the TextRun origin, including inline code's nonzero left padding.
        for (x, y) in [(5.44, 18.0), (7.82, 26.75), (31.5, 36.0)] {
            let exact = super::synthetic_italic_local_f64(x, y);
            let transformed_x = exact.sx * x + exact.kx * y + exact.tx;
            assert!((transformed_x - x).abs() < 1e-14);
            assert_eq!(exact.ky * x + exact.sy * y + exact.ty, y);
            let parsed = super::synthetic_italic_local_f32(x, y);
            assert!((exact.tx - f64::from(parsed.tx)).abs() < 1e-5);
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn expanded_alpha_passes_match_complete_original_go_buffers() {
        let cases: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/blur-expanded-source-v090.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let bounds = case["input"]["bounds"].as_array().unwrap();
            let mut layer = super::AlphaRegion {
                left: bounds[0].as_i64().unwrap() as isize,
                top: bounds[1].as_i64().unwrap() as isize,
                width: (bounds[2].as_i64().unwrap() - bounds[0].as_i64().unwrap()) as usize,
                height: (bounds[3].as_i64().unwrap() - bounds[1].as_i64().unwrap()) as usize,
                pixels: case["input"]["alpha"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_u64().unwrap() as u8)
                    .collect(),
            };
            for radius in case["radii"].as_array().unwrap() {
                let radius = radius.as_u64().unwrap() as usize;
                layer = super::box_blur_alpha_expanded(&layer, radius, true).unwrap();
                layer = super::box_blur_alpha_expanded(&layer, radius, false).unwrap();
            }
            let output = &case["output"];
            assert_eq!(
                [
                    layer.left,
                    layer.top,
                    layer.left + layer.width as isize,
                    layer.top + layer.height as isize
                ],
                std::array::from_fn(|i| output["bounds"][i].as_i64().unwrap() as isize),
                "{}",
                case["name"]
            );
            let expected: Vec<u8> = output["alpha"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_u64().unwrap() as u8)
                .collect();
            assert_eq!(layer.pixels, expected, "{}", case["name"]);
        }
    }

    #[test]
    fn shadow_alpha_retains_ink_bounds_and_expands_outside_the_page() {
        let mut rgba = vec![0; 200 * 150 * 4];
        rgba[3] = 255;
        let mut layer = super::occupied_alpha_region(&rgba, 200, 150).unwrap();
        assert_eq!(
            (layer.left, layer.top, layer.width, layer.height),
            (0, 0, 1, 1)
        );
        for radius in [2, 1, 1] {
            layer = super::box_blur_alpha_expanded(&layer, radius, true).unwrap();
            layer = super::box_blur_alpha_expanded(&layer, radius, false).unwrap();
        }
        assert_eq!(
            (layer.left, layer.top, layer.width, layer.height),
            (-4, -4, 9, 9)
        );
        // The original Go control's edge impulse returns alpha10 to (0,0).
        // Page-sized intermediate passes lose that off-page contribution.
        assert_eq!(layer.pixels[4 * layer.width + 4], 10);
        assert!(super::occupied_alpha_region(&[0; 4], 1, 1).is_none());
    }

    fn text_reference_witness(
        private: bool,
        masked: bool,
    ) -> (usvg::Tree, super::SvgImagePathSources) {
        let x = 63.0_f64;
        let y = 89.0_f64 + 53.0 / 3.0;
        let private_attributes = if private {
            format!(
                "data-d2-rs-text-x-bits=\"{:016x}\" data-d2-rs-text-y-bits=\"{:016x}\"",
                x.to_bits(),
                y.to_bits()
            )
        } else {
            String::new()
        };
        let mask_attribute = if masked { "mask=\"url(#mask)\"" } else { "" };
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="293" height="400">
            <defs><mask id="mask" maskUnits="userSpaceOnUse" x="0" y="0" width="293" height="400"><rect width="293" height="400" fill="white" fill-opacity="0.75"/></mask></defs>
            <g id="ordinary-ancestor" transform="translate(101 101)" {mask_attribute}>
              <g id="ordinary-parent"><text id="line-1" x="{x}" y="{y}" {private_attributes} font-family="Source Sans Pro" font-size="16" font-style="italic" text-anchor="middle">identical</text></g>
            </g></svg>"#
        );
        let (source, sources) = super::super::svg_precision::tag_embedded_svg_images(&source);
        let mut options = usvg::Options::default();
        super::super::load_bundled_fonts(&mut options);
        let tree =
            usvg::Tree::from_str(&source, &options).expect("real text reference witness parses");
        (tree, sources)
    }

    fn text_reference_parent(group: &usvg::Group) -> Option<(&usvg::Group, &usvg::Text)> {
        for child in group.children() {
            match child {
                usvg::Node::Text(text) if text.id() == "line-1" => return Some((group, text)),
                usvg::Node::Group(child) => {
                    if let Some(found) = text_reference_parent(child) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn paint_reference_contexts(
        tree: &usvg::Tree,
        paint_bounds: super::TextPaintBounds,
    ) -> std::collections::HashMap<String, super::PaintReferenceContext> {
        let mut contexts = std::collections::HashMap::new();
        super::collect_paint_reference_contexts(
            tree.root(),
            super::PaintReferenceContext {
                paint_bounds: Some(paint_bounds),
            },
            &mut contexts,
            &super::SvgImagePathSources::new(),
            tree,
            2.0,
        );
        contexts
    }

    #[test]
    fn raster_text_reference_private_plain_ancestors_use_canvas_bounds() {
        let (tree, sources) = text_reference_witness(true, false);
        let (parent, text) = text_reference_parent(tree.root()).unwrap();
        let canvas = super::TextPaintBounds {
            left: 0,
            top: 0,
            right: 586,
            bottom: 800,
        };
        let contexts = paint_reference_contexts(&tree, canvas);
        assert_eq!(
            contexts[text.id()].paint_bounds,
            Some(canvas),
            "ordinary ancestors retain the complete parent rectangle"
        );
        let legacy = super::group_reference_origin(parent, 2.0);
        assert!(
            legacy.x > 0.0 && legacy.y > 0.0,
            "actual prepared ink bounds differ from full-canvas reference"
        );
        let selected = super::text_reference_origin(
            legacy,
            sources.exact_text_origin_by_id(text.id()).is_some(),
            contexts.get(text.id()).copied(),
        );
        // Observed original Go TextRun label:1, first contour point. bands.go
        // retains canvas (0,0)..(586,800) even in actual band y=256..512.
        let source_point = Point64 {
            x: f64::from_bits(0x4071040000000000),
            y: f64::from_bits(0x4079f55555555556),
        };
        let mut rasterizer = Rasterizer::new(586, 800, 0.375);
        rasterizer.set_reference_origin(selected);
        let rounded = rasterizer.reference_point(source_point);
        assert_eq!(
            (rounded.y + selected.y).to_bits(),
            f64::from(f32::from_bits(0x43cfaaab)).to_bits(),
            "actual source corner must round relative to the inherited canvas, not the ordinary parent's ink"
        );
        assert_eq!((selected.x, selected.y), (0.0, 0.0));
    }

    #[test]
    fn raster_text_reference_private_preserves_source_crop_without_stroke_clearance() {
        let (tree, sources) = text_reference_witness(true, false);
        let (parent, text) = text_reference_parent(tree.root()).unwrap();
        // Source bands.go passes an effect's already-intersected visible
        // rectangle to every ordinary descendant. This supplied source crop
        // tests that contract; it does not qualify usvg effect-bound recovery.
        let crop = super::TextPaintBounds {
            left: 200,
            top: 300,
            right: 500,
            bottom: 700,
        };
        let contexts = paint_reference_contexts(&tree, crop);
        assert_eq!(contexts[text.id()].paint_bounds, Some(crop));
        let legacy = super::group_reference_origin(parent, 2.0);
        let stroke_reference = Point64 {
            x: legacy.x - 2.0,
            y: legacy.y,
        };
        let selected = super::text_reference_origin(
            stroke_reference,
            sources.exact_text_origin_by_id(text.id()).is_some(),
            contexts.get(text.id()).copied(),
        );
        assert_eq!(
            (selected.x, selected.y),
            (200.0, 300.0),
            "private text inherits source crop independently of stroke's mask clearance"
        );
        assert_eq!(contexts[text.id()].paint_bounds.unwrap().right, 500);
        assert_eq!(contexts[text.id()].paint_bounds.unwrap().bottom, 700);
    }

    #[test]
    fn raster_text_reference_untagged_preserves_legacy_reference() {
        let (tree, sources) = text_reference_witness(false, false);
        let (parent, text) = text_reference_parent(tree.root()).unwrap();
        let contexts = paint_reference_contexts(
            &tree,
            super::TextPaintBounds {
                left: 0,
                top: 0,
                right: 586,
                bottom: 800,
            },
        );
        assert!(sources.exact_text_origin_by_id(text.id()).is_none());
        let legacy = super::group_reference_origin(parent, 2.0);
        let selected =
            super::text_reference_origin(legacy, false, contexts.get(text.id()).copied());
        assert_eq!(
            (selected.x.to_bits(), selected.y.to_bits()),
            (legacy.x.to_bits(), legacy.y.to_bits())
        );
    }

    #[test]
    fn raster_text_reference_unqualified_mask_preserves_legacy_reference() {
        let (tree, sources) = text_reference_witness(true, true);
        let (parent, text) = text_reference_parent(tree.root()).unwrap();
        let contexts = paint_reference_contexts(
            &tree,
            super::TextPaintBounds {
                left: 0,
                top: 0,
                right: 586,
                bottom: 800,
            },
        );
        assert_eq!(
            contexts[text.id()].paint_bounds,
            None,
            "real mask boundary needs source prepared visible bounds; f32 ink bounds do not qualify it"
        );
        let legacy = super::group_reference_origin(parent, 2.0);
        let selected = super::text_reference_origin(
            legacy,
            sources.exact_text_origin_by_id(text.id()).is_some(),
            contexts.get(text.id()).copied(),
        );
        assert_eq!(
            (selected.x.to_bits(), selected.y.to_bits()),
            (legacy.x.to_bits(), legacy.y.to_bits())
        );
    }

    #[test]
    fn raster_canvas_reference_actual_native_wrapper_preserves_private_context() {
        use d2_rs_core::{
            Edge, EdgeLabel, Graph, LabelPosition, LayoutReport, LayoutResult, Node, Point, Rect,
            Score, Size,
        };
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
            Some(EdgeLabel {
                text: " identical \nidentical  duplicate\nlast line ".to_owned(),
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
            boxes: std::collections::BTreeMap::from([
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
            routes: std::collections::BTreeMap::from([(
                edge,
                vec![Point { x: 0.0, y: 100.0 }, Point { x: 200.0, y: 100.0 }],
            )]),
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
        let (native, _) = d2_rs_render::render_svg_with_raster_sources_checked(
            &graph,
            &result,
            &Default::default(),
            &Default::default(),
            d2_rs_render::RenderOptions::default(),
            false,
        )
        .unwrap();
        // Exercise the actual export preparation, including native italic IDs,
        // and usvg's conversion of the renderer's nested SVG into a clip group.
        let native = String::from_utf8(native).unwrap();
        let native = super::super::strip_embedded_font_faces(&native);
        let native = super::super::emulate_go_synthetic_italic(&native);
        let (native, sources) = super::super::svg_precision::tag_embedded_svg_images(&native);
        let mut options = usvg::Options::default();
        super::super::load_bundled_fonts(&mut options);
        let tree = usvg::Tree::from_str(&native, &options).unwrap();
        fn visit<'a>(
            group: &'a usvg::Group,
            sources: &super::SvgImagePathSources,
            viewport_seen: &mut bool,
            texts: &mut Vec<&'a usvg::Text>,
        ) {
            if sources.is_native_canvas_viewport(group.id()) && group.clip_path().is_some() {
                *viewport_seen = true;
            }
            for child in group.children() {
                match child {
                    usvg::Node::Group(child) => visit(child, sources, viewport_seen, texts),
                    usvg::Node::Text(text)
                        if sources.exact_text_origin_by_id(text.id()).is_some() =>
                    {
                        texts.push(text)
                    }
                    _ => {}
                }
            }
        }
        let mut viewport_seen = false;
        let mut texts = Vec::new();
        visit(tree.root(), &sources, &mut viewport_seen, &mut texts);
        assert!(
            viewport_seen,
            "actual native wrapper reaches usvg's synthetic viewport clip with its source identity"
        );
        assert_eq!(
            texts.len(),
            3,
            "actual renderer private lines survive export preparation"
        );
        let canvas = super::TextPaintBounds {
            left: 0,
            top: 0,
            right: i64::from(tree.size().to_int_size().width()) * 2,
            bottom: i64::from(tree.size().to_int_size().height()) * 2,
        };
        let mut contexts = std::collections::HashMap::new();
        super::collect_paint_reference_contexts(
            tree.root(),
            super::PaintReferenceContext {
                paint_bounds: Some(canvas),
            },
            &mut contexts,
            &sources,
            &tree,
            2.0,
        );
        for (text, original_line) in
            texts
                .into_iter()
                .zip([" identical ", "identical  duplicate", "last line "])
        {
            let shaped_text = text
                .layouted()
                .iter()
                .flat_map(|span| span.positioned_glyphs.iter())
                .map(|glyph| glyph.text.as_str())
                .collect::<String>();
            assert_eq!(
                shaped_text, original_line,
                "Source shapes the raw line; export parsing must retain every space"
            );
            assert_eq!(
                contexts[text.id()].paint_bounds,
                Some(canvas),
                "the native serialization viewport is ordinary Go scene ancestry, not an authored clip effect"
            );
        }
    }

    #[test]
    fn raster_canvas_reference_unmarked_full_frame_clip_remains_unqualified() {
        let source = r#"<svg xmlns="http://www.w3.org/2000/svg" width="293" height="400"><defs><clipPath id="real-clip"><rect width="293" height="400"/></clipPath></defs><g clip-path="url(#real-clip)"><text id="line-1" x="63" y="106.66666666666667" font-family="Source Sans Pro" font-size="16">identical</text></g></svg>"#;
        let mut options = usvg::Options::default();
        super::super::load_bundled_fonts(&mut options);
        let tree = usvg::Tree::from_str(source, &options).unwrap();
        let contexts = paint_reference_contexts(
            &tree,
            super::TextPaintBounds {
                left: 0,
                top: 0,
                right: 586,
                bottom: 800,
            },
        );
        assert_eq!(
            contexts["line-1"].paint_bounds, None,
            "covering the frame does not qualify a real unmarked clip effect"
        );
    }

    #[test]
    fn raster_canvas_reference_qualified_ancestor_preserves_real_descendant_effects() {
        for (name, attribute) in [
            ("clip", "clip-path=\"url(#real-clip)\""),
            ("mask", "mask=\"url(#real-mask)\""),
            ("filter", "filter=\"url(#real-filter)\""),
            ("opacity", "opacity=\"0.5\""),
            ("isolation", "style=\"isolation:isolate\""),
            ("blend", "style=\"mix-blend-mode:multiply\""),
        ] {
            let source = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="293" height="400">
                <defs>
                  <clipPath id="real-clip"><rect width="293" height="400"/></clipPath>
                  <mask id="real-mask"><rect width="293" height="400" fill="white"/></mask>
                  <filter id="real-filter"><feGaussianBlur stdDeviation="1"/></filter>
                </defs>
                <svg id="d2-rs-native-canvas-viewport-main" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="293" height="400">
                  <g id="real-descendant-{name}" {attribute}><text id="line-1" x="63" y="106.66666666666667" font-family="Source Sans Pro" font-size="16">identical</text></g>
                </svg></svg>"##
            );
            let (source, sources) = super::super::svg_precision::tag_embedded_svg_images(&source);
            assert!(sources.is_native_canvas_viewport("d2-rs-native-canvas-viewport-main"));
            let mut options = usvg::Options::default();
            super::super::load_bundled_fonts(&mut options);
            let tree = usvg::Tree::from_str(&source, &options).unwrap();
            let mut contexts = std::collections::HashMap::new();
            super::collect_paint_reference_contexts(
                tree.root(),
                super::PaintReferenceContext {
                    paint_bounds: Some(super::TextPaintBounds {
                        left: 0,
                        top: 0,
                        right: 586,
                        bottom: 800,
                    }),
                },
                &mut contexts,
                &sources,
                &tree,
                2.0,
            );
            let context = contexts.get("line-1").unwrap_or_else(|| {
                panic!("the real descendant {name} control must preserve its authored text node")
            });
            assert_eq!(
                context.paint_bounds, None,
                "a qualified viewport must not bypass a real descendant {name} effect"
            );
        }
    }

    #[test]
    fn bidi_controls_retain_the_native_fixed_text_path() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"256\" height=\"100\"><text x=\"128\" y=\"50\" font-family=\"Source Sans Pro\" font-size=\"16\" text-anchor=\"middle\">\u{202e}RTL override\u{202c}</text></svg>";
        let mut options = usvg::Options::default();
        super::super::load_bundled_fonts(&mut options);
        let tree = usvg::Tree::from_str(svg, &options).unwrap();
        let usvg::Node::Text(text) = &tree.root().children()[0] else {
            panic!("text");
        };
        assert!(super::native_mixed_font_layout(&tree, text).is_some());
        let mut pixels = vec![0; 256 * 100 * 4];
        assert_eq!(
            super::draw_text_fixed(
                text,
                &tree,
                &mut pixels,
                256,
                100,
                1.0,
                1.0,
                Point64::default(),
                None,
                None,
                Point64::default(),
                false,
                &super::SvgImagePathSources::default(),
            ),
            Some(true)
        );
        assert!(pixels.iter().any(|&value| value != 0));
    }

    fn fixed_text_transforms_for_source(body: &str) -> Vec<Vec<super::Affine64>> {
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="500" height="200"><text font-family="Source Sans Pro" font-size="16" text-anchor="middle">{body}</text></svg>"#
        );
        let mut options = usvg::Options::default();
        super::super::load_bundled_fonts(&mut options);
        let tree = usvg::Tree::from_str(&source, &options).expect("text fixture parses");
        let text = tree
            .root()
            .children()
            .iter()
            .find_map(|node| match node {
                usvg::Node::Text(text) => Some(text.as_ref()),
                _ => None,
            })
            .expect("fixture retains a text node");
        super::fixed_glyph_transforms(&tree, text, text.layouted())
            .expect("each authored chunk maps to its source style spans")
    }

    fn corrected_text_origin_witness(body: &str, private: bool) -> Vec<Vec<super::Affine64>> {
        let origin = super::SvgPoint {
            x: 63.0,
            // Original centeredTextRuns for baseline89, height53, 3lines.
            y: 89.0 + 53.0 / 3.0,
        };
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="500" height="200"><text x="{}" y="{}" font-family="Source Sans Pro" font-size="16" text-anchor="middle">{body}</text></svg>"#,
            origin.x, origin.y
        );
        let mut options = usvg::Options::default();
        super::super::load_bundled_fonts(&mut options);
        let tree = usvg::Tree::from_str(&source, &options).expect("text witness parses");
        let text = tree
            .root()
            .children()
            .iter()
            .find_map(|node| match node {
                usvg::Node::Text(text) => Some(text.as_ref()),
                _ => None,
            })
            .expect("witness keeps a real text node");
        let mut transforms = super::fixed_glyph_transforms(&tree, text, text.layouted()).unwrap();
        super::apply_exact_text_origin(&mut transforms, text, origin, private).unwrap();
        transforms
    }

    #[test]
    fn raster_text_origin_private_baseline_matches_exact_scene_bits() {
        let transforms = corrected_text_origin_witness("identical", true);
        let expected_y = (89.0_f64 + 53.0 / 3.0).to_bits();
        let glyphs = transforms.iter().flatten().collect::<Vec<_>>();
        assert!(!glyphs.is_empty());
        for glyph in glyphs {
            assert_eq!(glyph.ty.to_bits(), expected_y, "zero-offset line baseline");
        }
    }

    #[test]
    fn raster_text_origin_private_preserves_relative_baseline_shift() {
        let transforms =
            corrected_text_origin_witness(r#"<tspan baseline-shift="3">up</tspan>plain"#, true);
        assert_eq!(transforms.len(), 2, "actual style span boundary");
        let origin_y = 89.0_f64 + 53.0 / 3.0;
        for glyph in &transforms[0] {
            assert_eq!(glyph.ty.to_bits(), (origin_y - 3.0).to_bits());
        }
        for glyph in &transforms[1] {
            assert_eq!(glyph.ty.to_bits(), origin_y.to_bits());
        }
    }

    #[test]
    fn raster_text_origin_legacy_baseline_behavior_is_preserved() {
        let transforms = corrected_text_origin_witness("identical", false);
        let expected_y = (89.0_f64 + 53.0 / 3.0).to_bits();
        for glyph in transforms.iter().flatten() {
            assert_eq!(glyph.ty.to_bits(), expected_y);
        }
    }

    fn assert_fixed_text_origins_equal(
        actual: &[Vec<super::Affine64>],
        expected: &[Vec<super::Affine64>],
    ) {
        let actual = actual.iter().flatten().collect::<Vec<_>>();
        let expected = expected.iter().flatten().collect::<Vec<_>>();
        assert_eq!(actual.len(), expected.len(), "glyph count");
        for (index, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
            assert_eq!(
                (actual.tx.to_bits(), actual.ty.to_bits()),
                (expected.tx.to_bits(), expected.ty.to_bits()),
                "glyph {index} must retain its own independently anchored text chunk"
            );
        }
    }

    #[test]
    fn fixed_text_multiline_chunks_match_independent_line_origins() {
        // D2 v0.9.0 centeredTextRuns emits one AnchorMiddle TextRun per
        // newline. Different line widths must not influence each other's
        // fixed-point anchor, even when their authored X coordinate is equal.
        for (first, second) in [
            ("A much longer first line", "short"),
            ("short", "A much longer second line"),
            ("identical", "identical"),
        ] {
            let first = format!(r#"<tspan x="137.25" y="24">{first}</tspan>"#);
            let second = format!(r#"<tspan x="137.25" y="42.5">{second}</tspan>"#);
            let mut expected = fixed_text_transforms_for_source(&first);
            expected.extend(fixed_text_transforms_for_source(&second));
            let actual = fixed_text_transforms_for_source(&format!("{first}{second}"));
            assert_fixed_text_origins_equal(&actual, &expected);
        }
    }

    #[test]
    fn fixed_text_multiline_preserves_styled_spans_within_each_chunk() {
        // A style boundary remains inside one anchored chunk. Splitting by
        // every layouted span would independently center the bold substring.
        let first = r#"<tspan x="137.25" y="24">AV<tspan font-weight="bold"> longer bold </tspan>end</tspan>"#;
        let second =
            r#"<tspan x="137.25" y="42.5"><tspan font-weight="bold">short</tspan> tail</tspan>"#;
        let first_expected = fixed_text_transforms_for_source(first);
        let second_expected = fixed_text_transforms_for_source(second);
        assert!(first_expected.len() > 1 && second_expected.len() > 1);
        let mut expected = first_expected;
        expected.extend(second_expected);
        let actual = fixed_text_transforms_for_source(&format!("{first}{second}"));
        assert_fixed_text_origins_equal(&actual, &expected);
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn imported_svg_opacity_matches_complete_original_go_frames() {
        use base64::Engine as _;
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/imported-svg-opacity-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 24);
        let mut failures = Vec::new();
        for (index, row) in rows.as_array().unwrap().iter().enumerate() {
            let authored = row["svg"].as_str().unwrap();
            let named = authored.replace("<image ", "<image id=\"authored-image\" ");
            let input = if index % 2 == 0 { authored } else { &named };
            let (svg, sources) = crate::svg_precision::tag_embedded_svg_images(input);
            let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
            let (width, height, pixels) = super::render_simple_with_sources(
                &tree,
                row["scale"].as_u64().unwrap() as u32,
                &[],
                &sources,
            )
            .unwrap();
            assert_eq!(
                (u64::from(width), u64::from(height)),
                (
                    row["width"].as_u64().unwrap(),
                    row["height"].as_u64().unwrap()
                )
            );
            let expected = base64::engine::general_purpose::STANDARD
                .decode(row["pixels"].as_str().unwrap())
                .unwrap();
            let different: Vec<_> = pixels
                .chunks_exact(4)
                .zip(expected.chunks_exact(4))
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .collect();
            if !different.is_empty() {
                failures.push(format!(
                    "frame {index}: {} pixels; first {:?}",
                    different.len(),
                    different[0]
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn nested_svg_path_transform_precedes_image_placement() {
        let path = Transform::from_row(
            0.992_546_14,
            0.121_869_34,
            -0.121_869_34,
            0.992_546_14,
            4.263_342,
            -3.411_296,
        );
        let image = Transform::from_translate(100.0, 100.0);
        let combined = path_scene_transform(path, Some(image));

        let mut expected = Point::from_xy(8.0, 32.0);
        path.map_point(&mut expected);
        image.map_point(&mut expected);
        let mut actual = Point::from_xy(8.0, 32.0);
        combined.map_point(&mut actual);

        assert!((actual.x - expected.x).abs() < 1e-5);
        assert!((actual.y - expected.y).abs() < 1e-5);
        assert!((actual.x - 108.304).abs() < 0.01);
        assert!((actual.y - 129.325).abs() < 0.01);
    }

    #[test]
    fn clean_stroke_points_matches_go_closed_and_open_paths() {
        let a = Point64 { x: 1.0, y: 2.0 };
        let b = Point64 { x: 3.0, y: 4.0 };
        let c = Point64 { x: 5.0, y: 6.0 };
        let closed = [a, b, b, c, a];
        let open = [a, b, b, c, a];

        assert!(clean_stroke_points(&closed, true) == [a, b, c]);
        assert!(clean_stroke_points(&open, false) == [a, b, c, a]);
    }

    #[test]
    fn reference_rounding_origin_is_distinct_from_layer_origin() {
        let mut rasterizer = Rasterizer::new(128, 128, 0.125);
        rasterizer.set_reference_origin(Point64 {
            x: 3557.0,
            y: 624.0,
        });
        rasterizer.set_rounding_origin(Point64 {
            x: 3559.0,
            y: 624.0,
        });

        let point = Point64 {
            x: 3_622.586_734_771_728_5,
            y: 995.398_048_400_878_9,
        };
        let local = rasterizer.reference_point(point);
        let upstream_x = f64::from((point.x - 3559.0) as f32) + 2.0;
        let layer_only_x = f64::from((point.x - 3557.0) as f32);

        assert_eq!(local.x.to_bits(), upstream_x.to_bits());
        assert_ne!(local.x.to_bits(), layer_only_x.to_bits());
        assert_eq!(
            local.y.to_bits(),
            f64::from((point.y - 624.0) as f32).to_bits()
        );
    }

    #[test]
    fn reference_origin_includes_conservative_miter_stroke_extent() {
        fn find_group<'a>(group: &'a usvg::Group, id: &str) -> Option<&'a usvg::Group> {
            if group.id() == id {
                return Some(group);
            }
            group.children().iter().find_map(|child| match child {
                usvg::Node::Group(child) => find_group(child, id),
                usvg::Node::Path(_) | usvg::Node::Image(_) | usvg::Node::Text(_) => None,
            })
        }

        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40" viewBox="0 0 40 40">
            <g id="miter"><path d="M 10 20 L 20 20" fill="none" stroke="#000" stroke-width="2" stroke-linecap="butt" stroke-linejoin="miter" stroke-miterlimit="4"/></g>
            <g id="round"><path d="M 10 20 L 20 20" fill="none" stroke="#000" stroke-width="2" stroke-linecap="butt" stroke-linejoin="round"/></g>
        </svg>"##;
        let tree = usvg::Tree::from_str(source, &usvg::Options::default()).unwrap();
        let miter = find_group(tree.root(), "miter").unwrap();
        let round = find_group(tree.root(), "round").unwrap();

        assert_eq!(group_reference_origin(miter, 2.0).x, 11.0);
        assert_eq!(group_reference_origin(round, 2.0).x, 17.0);
    }

    #[test]
    fn direct_svg_path_ink_bounds_distinguish_viewport_overflow() {
        let inside = usvg::Tree::from_str(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10">
              <path d="M 1 1 H 9 V 9 H 1 Z"/>
            </svg>"##,
            &usvg::Options::default(),
        )
        .unwrap();
        let outside = usvg::Tree::from_str(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10">
              <path d="M -1 1 H 9 V 9 H -1 Z"/>
            </svg>"##,
            &usvg::Options::default(),
        )
        .unwrap();

        assert!(direct_svg_group_fits_viewport(&inside));
        assert!(!direct_svg_group_fits_viewport(&outside));
    }

    #[test]
    fn embedded_vector_overflow_keeps_display_scale_and_intrinsic_clip() {
        use base64::Engine as _;
        // Source clips the intrinsic asset and composes its display mapping.
        // An effect inside the asset exercises the general subtree path;
        // the effect-free variant exercises analytic overflow clipping.
        for opacity in ["1", "0.5"] {
            let asset = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg"
                width="20" height="20" viewBox="0 0 20 20">
                <g opacity="{opacity}"><path d="M -5 -5 H 25 V 25 H -5 Z" fill="red"/></g>
            </svg>"##
            );
            let data = base64::engine::general_purpose::STANDARD.encode(asset);
            for (display_width, display_height) in [(8, 8), (16, 8)] {
                let source = format!(
                    r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
                    <image x="4" y="4" width="{display_width}" height="{display_height}"
                        preserveAspectRatio="none" href="data:image/svg+xml;base64,{data}"/>
                </svg>"##
                );
                let tree = usvg::Tree::from_str(&source, &usvg::Options::default()).unwrap();
                let (width, height, pixels) = render_simple(&tree, 1).unwrap();
                for y in 0..height as usize {
                    for x in 0..width as usize {
                        let offset = (y * width as usize + x) * 4;
                        let expected = if (4..4 + display_width).contains(&x)
                            && (4..4 + display_height).contains(&y)
                        {
                            if opacity == "1" {
                                [255, 0, 0, 255]
                            } else {
                                [255, 127, 127, 255]
                            }
                        } else {
                            [255; 4]
                        };
                        assert_eq!(
                            &pixels[offset..offset + 4],
                            expected,
                            "opacity={opacity} display={display_width}x{display_height} at {x},{y}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn opacity_layer_preserves_stroke_ink_outside_geometry_bounds() {
        let tree = usvg::Tree::from_str(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <g opacity="0.5"><rect x="10" y="10" width="20" height="20"
                fill="none" stroke="red" stroke-width="4"/></g>
            </svg>"##,
            &usvg::Options::default(),
        )
        .unwrap();
        let (width, _, pixels) = render_simple(&tree, 1).unwrap();
        for (x, y) in [(8, 20), (20, 8), (31, 20), (20, 31)] {
            let offset = (y * width as usize + x) * 4;
            assert_eq!(
                &pixels[offset..offset + 4],
                &[255, 127, 127, 255],
                "stroke ink at {x},{y} lies outside the unpainted rectangle bounds"
            );
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn masked_sketch_blends_match_complete_original_go_frames() {
        use base64::Engine as _;
        let source: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sketch-masked-blends-source-v090.json"
        ))
        .unwrap();
        let rows = source.as_array().unwrap();
        assert_eq!(rows.len(), 15);
        for row in rows {
            let mode = row["mode"].as_str().unwrap();
            let opacity = row["opacity"].as_f64().unwrap();
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><defs><mask id="hole" maskUnits="userSpaceOnUse" x="0" y="0" width="8" height="8"><rect width="8" height="8" fill="white"/><rect x="2" width="4" height="8" fill="black" fill-opacity="0.75"/></mask></defs><rect width="8" height="8" fill="#40c0ff"/><g id="blended" mask="url(#hole)" style="mix-blend-mode:{mode}" opacity="{opacity}" data-d2-rs-exact-opacity="{opacity}"><rect width="8" height="8" fill="#8040ff" fill-opacity="{}"/></g></svg>"##,
                128.0 / 255.0
            );
            let (svg, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
            let (width, height, pixels) =
                super::render_simple_with_sources(&tree, 1, &[], &sources)
                    .expect("mask and blend stay native");
            assert_eq!((width, height), (8, 8));
            let expected = base64::engine::general_purpose::STANDARD
                .decode(row["pixels"].as_str().unwrap())
                .unwrap();
            assert_eq!(pixels, expected, "mode={mode} opacity={opacity}");
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_even_odd_streak_tiles_match_original_go_at_both_scales() {
        use sha2::{Digest, Sha256};
        let source: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sketch-streak-tiles-source-v090.json"
        ))
        .unwrap();
        let rows = source.as_array().unwrap();
        assert_eq!(rows.len(), 16);
        for row in rows {
            let svg = match row["category"].as_str().unwrap() {
                "bright" => d2_rs_testdata::oracle_fixture!(
                    "tests/fixtures/sketch-streak-bright-source-v090.svg"
                ),
                "normal" => d2_rs_testdata::oracle_fixture!(
                    "tests/fixtures/sketch-streak-normal-source-v090.svg"
                ),
                "dark" => d2_rs_testdata::oracle_fixture!(
                    "tests/fixtures/sketch-streak-dark-source-v090.svg"
                ),
                "darker" => d2_rs_testdata::oracle_fixture!(
                    "tests/fixtures/sketch-streak-darker-source-v090.svg"
                ),
                _ => panic!("unexpected source category"),
            };
            let (svg, sources) = crate::svg_precision::tag_embedded_svg_images(svg);
            let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
            let (width, height, pixels) = super::render_simple_with_sources_and_background(
                &tree,
                row["scale"].as_u64().unwrap() as u32,
                &[],
                &sources,
                if row["background"] == "transparent" {
                    [0; 4]
                } else {
                    [255; 4]
                },
            )
            .expect("native even-odd tile; fallback cannot pass");
            assert_eq!(
                (u64::from(width), u64::from(height)),
                (
                    row["width"].as_u64().unwrap(),
                    row["height"].as_u64().unwrap()
                )
            );
            assert_eq!(
                format!("{:x}", Sha256::digest(&pixels)),
                row["sha256"].as_str().unwrap(),
                "{row}"
            );
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn sketch_blend_modes_match_all_original_go_pixel_controls() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sketch-blends-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 90);
        for row in rows {
            let bytes = |name: &str| -> Vec<u8> {
                row[name]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_u64().unwrap() as u8)
                    .collect()
            };
            let mode = match row["mode"].as_str().unwrap() {
                "multiply" => usvg::BlendMode::Multiply,
                "darken" => usvg::BlendMode::Darken,
                "color-burn" => usvg::BlendMode::ColorBurn,
                "overlay" => usvg::BlendMode::Overlay,
                "lighten" => usvg::BlendMode::Lighten,
                _ => panic!("unexpected source mode"),
            };
            let mut destination = bytes("backdrop");
            super::composite_effect_group(
                &mut destination,
                &bytes("source"),
                1,
                1,
                resvg::tiny_skia::Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap(),
                &super::LayerRegion::full(1, 1),
                1.0,
                row["opacity"].as_f64().unwrap(),
                mode,
            );
            assert_eq!(destination, bytes("result"), "{row}");
        }
    }
    #[test]
    fn multiply_blend_group_matches_go_layer_compositing() {
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
          <rect width="8" height="8" fill="white"/>
          <g style="mix-blend-mode:multiply;opacity:0.5">
            <rect width="8" height="8" fill="red"/>
          </g>
        </svg>"##;
        let tree = usvg::Tree::from_str(source, &usvg::Options::default()).unwrap();
        let (width, _, pixels) = render_simple(&tree, 1).expect("isolated multiply group");
        let index = (4 * width as usize + 4) * 4;
        assert_eq!(&pixels[index..index + 4], &[255, 127, 127, 255]);
    }

    #[test]
    fn structured_streak_rect_keeps_scene_fractional_coverage() {
        // Pinned d2raster receives the structured header as an f64 scene
        // rectangle. At its bottom-right pixel, 1/2 * 1/3 coverage rounds to
        // 43; lowering the bottom coordinate through f32 rounds it to 42.
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4" viewBox="0 0 4 4">
          <svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="4" height="4" viewBox="0 0 4 4">
            <defs><pattern id="d2-rs-sketch-streak-darker" width="100" height="100" patternUnits="userSpaceOnUse"><rect width="100" height="100" fill="white" fill-opacity="0.058823529411764705"/></pattern></defs>
            <rect width="4" height="4" fill="#170206"/>
            <g style="mix-blend-mode:lighten"><rect id="control-native-class-0-streak-header" x="0.5" y="1" width="2" height="1.3333333333333333" fill="url(#d2-rs-sketch-streak-darker)" stroke="none"/></g>
          </svg>
        </svg>"##;
        let (source, sources) = crate::svg_precision::tag_embedded_svg_images(source);
        let tree = usvg::Tree::from_str(&source, &usvg::Options::default()).unwrap();
        let (width, height, pixels) = super::render_simple_with_sources(&tree, 1, &[], &sources)
            .expect("structured streak stays on the native scene path");
        assert_eq!((width, height), (4, 4));
        let offset = (2 * width as usize + 2) * 4;
        assert_eq!(&pixels[offset..offset + 4], &[26, 5, 9, 255]);
    }

    #[test]
    fn paper_pattern_vector_tile_is_supported_by_native_scanline() {
        let pattern = d2_rs_render::assets::PAPER_SVG.replace("%[1]s", "paper-probe");
        let source = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"75\" height=\"75\" viewBox=\"0 0 75 75\"><defs>{pattern}</defs><rect width=\"75\" height=\"75\" fill=\"url(#paper-paper-probe)\"/></svg>"
        );
        let tree = usvg::Tree::from_str(&source, &usvg::Options::default())
            .expect("pinned paper pattern SVG parses");
        let pattern = tree
            .root()
            .children()
            .iter()
            .find_map(|node| match node {
                usvg::Node::Path(path) => path.fill().and_then(|fill| match fill.paint() {
                    usvg::Paint::Pattern(pattern) => Some(pattern.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .expect("paper fill resolves to a pattern");
        let width = 150_usize;
        let height = 150_usize;
        let mut pixels = vec![0_u8; width * height * 4];
        let raster_sources = super::RasterSourceMap::default();
        let image_paths = super::SvgImagePathSources::new();
        let rendered = super::draw_group(
            &tree,
            pattern.root(),
            &mut pixels,
            width,
            height,
            2.0,
            1.0,
            false,
            &raster_sources,
            &image_paths,
        );
        assert!(
            rendered,
            "paper pattern vector tree is supported by scanline"
        );
        let output =
            super::render_simple_with_sources_and_background(&tree, 2, &[], &image_paths, [255; 4])
                .expect("pattern-filled rectangle is supported by the scanline path");
        assert_eq!((output.0, output.1), (150, 150));
        let center = (75 * output.0 as usize + 75) * 4;
        assert!(
            output.2[center] < 255,
            "paper tile must contribute at the center; got {:?}",
            &output.2[center..center + 4]
        );
        if let Some(path) = std::env::var_os("D2_RS_DUMP_PAPER_PATTERN_TILE") {
            std::fs::write(path, pixels).expect("write paper pattern tile probe");
        }
    }

    #[test]
    fn simple_user_space_pattern_is_sampled_by_native_scanline() {
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
          <defs><pattern id="p" x="0" y="0" width="2" height="2" patternUnits="userSpaceOnUse">
            <rect width="2" height="2" fill="#123456"/>
          </pattern></defs>
          <rect width="8" height="8" fill="url(#p)"/>
        </svg>"##;
        let tree = usvg::Tree::from_str(source, &usvg::Options::default()).unwrap();
        let (width, _, pixels) = render_simple(&tree, 1).expect("user-space pattern");
        let pixel = |x: usize, y: usize| &pixels[(y * width as usize + x) * 4..][..4];
        assert_eq!(pixel(0, 0), &[0x12, 0x34, 0x56, 255]);
        assert_eq!(pixel(5, 6), &[0x12, 0x34, 0x56, 255]);
    }

    #[test]
    fn png_image_bilinear_sampling_keeps_source_8_and_16_bit_precision() {
        // Actual pinned Go image.go oracle: two pixels expanded to four.
        for (png, expected) in [
            (
                "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAFElEQVR4nAAHAPj/Av8AAAAA/wMABw4CAZBteiQAAAAASUVORK5CYII=",
                [
                    [255, 0, 0, 255],
                    [191, 0, 64, 255],
                    [64, 0, 191, 255],
                    [0, 0, 255, 255],
                ],
            ),
            (
                "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABEAYAAACksqPJAAAAHklEQVR4nAARAO7/Au7uERF3dxKBMzO7u6qq/t8DAEFFCI+7cWxBAAAAAElFTkSuQmCC",
                [
                    [254, 238, 246, 255],
                    [204, 225, 227, 255],
                    [102, 200, 189, 255],
                    [52, 187, 170, 255],
                ],
            ),
        ] {
            let source = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="1"><image width="4" height="1" preserveAspectRatio="none" href="data:image/png;base64,{png}"/></svg>"#
            );
            let tree = usvg::Tree::from_str(&source, &usvg::Options::default()).unwrap();
            let (width, height, pixels) = render_simple(&tree, 1).expect("Source PNG primitive");
            assert_eq!((width, height), (4, 1));
            assert_eq!(pixels, expected.into_iter().flatten().collect::<Vec<u8>>());
        }
    }

    #[test]
    fn png_image_coverage_rotation_reflection_and_group_opacity_match_source() {
        for (index, (png, transform, width, height, expected)) in [(r#"iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAFElEQVR4nAAHAPj/Av8AAAAA/wMABw4CAZBteiQAAAAASUVORK5CYII="#, r#"transform="matrix(1 0 0 1 0.25 0.25)""#, 5, 2, vec![[255,112,112,255],[231,64,88,255],[136,64,184,255],[64,64,255,255],[207,207,255,255],[255,207,207,255],[247,191,199,255],[215,191,231,255],[191,191,255,255],[239,239,255,255]]),
(r#"iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAFElEQVR4nAAHAPj/Av8AAAAA/wMABw4CAZBteiQAAAAASUVORK5CYII="#, r#"transform="matrix(0 1 -1 0 1 0)""#, 1, 4, vec![[255,0,0,255],[191,0,64,255],[64,0,191,255],[0,0,255,255]]),
(r#"iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAFElEQVR4nAAHAPj/Av8AAAAA/wMABw4CAZBteiQAAAAASUVORK5CYII="#, r#"transform="matrix(-1 0 0 1 4 0)""#, 4, 1, vec![[0,0,255,255],[64,0,191,255],[191,0,64,255],[255,0,0,255]]),
(r#"iVBORw0KGgoAAAANSUhEUgAAAAIAAAABEAYAAACksqPJAAAAHklEQVR4nAARAO7/Au7uERF3dxKBMzO7u6qq/t8DAEFFCI+7cWxBAAAAAElFTkSuQmCC"#, r#""#, 4, 1, vec![[255,247,251,255],[229,240,241,255],[178,227,222,255],[154,221,213,255]])].into_iter().enumerate() {
            let opacity = if index == 3 { "0.5" } else { "1" };
            let source = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}"><g opacity="{opacity}"><image width="4" height="1" preserveAspectRatio="none" {transform} href="data:image/png;base64,{png}"/></g></svg>"#);
            let tree = usvg::Tree::from_str(&source, &usvg::Options::default()).unwrap();
            let (_, _, pixels) = render_simple(&tree, 1).expect("Source transformed PNG");
            assert_eq!(pixels, expected.into_iter().flatten().collect::<Vec<u8>>(), "Source case {index}");
        }
    }

    #[test]
    fn native_effect_opacity_keeps_source_float64_half_byte_boundary() {
        // Original Go d2raster image .9 control, not a rounded f32 oracle.
        let source = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="1"><g id="native-effect" opacity="0.9" data-d2-rs-exact-opacity="0.9"><image width="4" height="1" preserveAspectRatio="none" href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAABEAYAAACksqPJAAAAHklEQVR4nAARAO7/Au7uERF3dxKBMzO7u6qq/t8DAEFFCI+7cWxBAAAAAElFTkSuQmCC"/></g></svg>"#;
        let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(source);
        let tree = usvg::Tree::from_str(&tagged, &usvg::Options::default()).unwrap();
        let (_, _, pixels) = super::render_simple_with_sources(&tree, 1, &[], &sources).unwrap();
        assert_eq!(
            pixels,
            [
                [254, 240, 247, 255],
                [209, 228, 230, 255],
                [117, 205, 195, 255],
                [72, 194, 178, 255]
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<u8>>()
        );
    }

    #[test]
    fn native_dashed_rect_matches_original_go_whole_rgba_controls() {
        use sha2::{Digest, Sha256};
        let mut failures = Vec::new();
        for (
            radius,
            opacity,
            logical_width,
            logical_height,
            scale,
            center,
            expected_width,
            expected_height,
            expected,
        ) in [
            (
                0.0,
                1.0,
                342.0,
                282.0,
                1,
                false,
                342,
                282,
                "fff4c3f179b9647d23f2425bbfcfa194fc44c166e9a7f49b07829129b55c56b3",
            ),
            (
                0.0,
                1.0,
                342.0,
                282.0,
                1,
                true,
                342,
                282,
                "fff4c3f179b9647d23f2425bbfcfa194fc44c166e9a7f49b07829129b55c56b3",
            ),
            (
                0.0,
                1.0,
                428.0,
                353.0,
                2,
                false,
                856,
                706,
                "7b9cbf5a46cf68c18083fff548cfe44d24e2b8b18423e6f86be2020c6488db53",
            ),
            (
                0.0,
                1.0,
                428.0,
                353.0,
                2,
                true,
                856,
                706,
                "f1b387ed76cd54cb50e0f82f9a7f36ec881a31da9dd15f58e9f97cf75cc02a94",
            ),
            (
                0.0,
                1.0,
                114.0,
                94.0,
                2,
                false,
                228,
                188,
                "8da35c4ee476b26dd5d8367a2341fa1d97672a53d68e94f0303c887969b1f1d3",
            ),
            (
                0.0,
                1.0,
                114.0,
                94.0,
                2,
                true,
                228,
                188,
                "8da35c4ee476b26dd5d8367a2341fa1d97672a53d68e94f0303c887969b1f1d3",
            ),
            (
                0.0,
                0.6,
                342.0,
                282.0,
                1,
                false,
                342,
                282,
                "33c88d76831d83ce557c1e8c60334a3960791400b396571093c76cb2713272a2",
            ),
            (
                0.0,
                0.6,
                342.0,
                282.0,
                1,
                true,
                342,
                282,
                "33c88d76831d83ce557c1e8c60334a3960791400b396571093c76cb2713272a2",
            ),
            (
                0.0,
                0.6,
                428.0,
                353.0,
                2,
                false,
                856,
                706,
                "9fc9a01e23e876558a60af420034acd44634afc0d835550badd5528981eb42d6",
            ),
            (
                0.0,
                0.6,
                428.0,
                353.0,
                2,
                true,
                856,
                706,
                "0452ab04f6466a6f4284d832807d9a466351f73edf32a8bb17f4f8aac6ee793f",
            ),
            (
                0.0,
                0.6,
                114.0,
                94.0,
                2,
                false,
                228,
                188,
                "8650b8e0268debeeb7069dc92d991d11f9851322c27df967b065fa23d414ae06",
            ),
            (
                0.0,
                0.6,
                114.0,
                94.0,
                2,
                true,
                228,
                188,
                "8650b8e0268debeeb7069dc92d991d11f9851322c27df967b065fa23d414ae06",
            ),
            (
                7.75,
                1.0,
                342.0,
                282.0,
                1,
                false,
                342,
                282,
                "509c794a23e9546c85dce3d609e73dbe6b7226044ba77585cf36dba50a2d48cf",
            ),
            (
                7.75,
                1.0,
                342.0,
                282.0,
                1,
                true,
                342,
                282,
                "509c794a23e9546c85dce3d609e73dbe6b7226044ba77585cf36dba50a2d48cf",
            ),
            (
                7.75,
                1.0,
                428.0,
                353.0,
                2,
                false,
                856,
                706,
                "8b5687c322662cc63e0bd9f1bf290732572ad373f22943a90399eb11f6fda339",
            ),
            (
                7.75,
                1.0,
                428.0,
                353.0,
                2,
                true,
                856,
                706,
                "084d06a4b29ab3df71912aec4703bc7e9d090c0cf439d5930639e0296a7a5bb4",
            ),
            (
                7.75,
                1.0,
                114.0,
                94.0,
                2,
                false,
                228,
                188,
                "a6a3c81300387137fa6f9262d6a76950bed22569f0f6a04956df836ff04d8395",
            ),
            (
                7.75,
                1.0,
                114.0,
                94.0,
                2,
                true,
                228,
                188,
                "a6a3c81300387137fa6f9262d6a76950bed22569f0f6a04956df836ff04d8395",
            ),
            (
                7.75,
                0.6,
                342.0,
                282.0,
                1,
                false,
                342,
                282,
                "a92494c5916b4c758adffd4057ff1c660d807912691854733ebf20b2c64fbcff",
            ),
            (
                7.75,
                0.6,
                342.0,
                282.0,
                1,
                true,
                342,
                282,
                "a92494c5916b4c758adffd4057ff1c660d807912691854733ebf20b2c64fbcff",
            ),
            (
                7.75,
                0.6,
                428.0,
                353.0,
                2,
                false,
                856,
                706,
                "4825604b67e2d21425fb77858422a089912c76335134376499b5781a953a4172",
            ),
            (
                7.75,
                0.6,
                428.0,
                353.0,
                2,
                true,
                856,
                706,
                "ac1f384fa88efeff29e871ef3da19662cbfabc5b71f3707ea3515aad6e1e822d",
            ),
            (
                7.75,
                0.6,
                114.0,
                94.0,
                2,
                false,
                228,
                188,
                "b11441d4c31893cd37a34f1acd187f84630eaf559480731b1a92dc20ce610f7c",
            ),
            (
                7.75,
                0.6,
                114.0,
                94.0,
                2,
                true,
                228,
                188,
                "b11441d4c31893cd37a34f1acd187f84630eaf559480731b1a92dc20ce610f7c",
            ),
        ] {
            let align = if center { "xMidYMid" } else { "xMinYMin" };
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{logical_width}" height="{logical_height}" viewBox="0 0 342 282" preserveAspectRatio="{align} meet"><svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="342" height="282" viewBox="-101 -101 342 282"><g opacity="{opacity}" data-d2-rs-dash-owner-id="control-native-raster-shape-0" data-d2-rs-dash-size-bits="4024000000000000" data-d2-rs-dash-gap-bits="4023bb3511f45fb3" data-d2-rs-dash-offset-bits="0000000000000000"><rect id="control-native-raster-shape-0-primitive-0" x="4.25" y="7.125" width="140.5" height="80.75" rx="{radius}" ry="{radius}" fill="#f7f8fe" stroke="#0d32b2" stroke-width="2" stroke-linejoin="round" stroke-dasharray="10 9.865639267998494"/></g></svg></svg>"##
            );
            let exact = crate::collect_exact_dash_parameters(&svg);
            assert_eq!(exact.len(), 1);
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let tree = usvg::Tree::from_str(&tagged, &usvg::Options::default()).unwrap();
            let (width, height, pixels) =
                super::render_simple_with_sources_and_background_and_dash_parameters(
                    &tree,
                    scale,
                    &[],
                    &sources,
                    [255; 4],
                    &exact,
                )
                .unwrap();
            assert_eq!((width, height), (expected_width, expected_height));
            let digest = format!("{:x}", Sha256::digest(&pixels));
            if digest != expected {
                failures.push(format!("radius{radius} opacity{opacity} {logical_width}x{logical_height} scale{scale} center{center}: native {digest} Source {expected}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    #[rustfmt::skip]
    fn native_dashed_rect_retains_interpolated_corner_endpoints_go_rgba_controls() {
        use sha2::{Digest, Sha256};
        let mut failures = Vec::new();
        for (radius, opacity, logical_width, logical_height, scale, center, expected_width, expected_height, expected) in [
            (0.0, 1.0, 342.0, 282.0, 1, false, 342, 282, "5e7ddf98a17eb21fcf00cdb81271a94f7c648f88f8df712f1d6ecf1a1ea1190f"),
            (0.0, 1.0, 342.0, 282.0, 1, true, 342, 282, "5e7ddf98a17eb21fcf00cdb81271a94f7c648f88f8df712f1d6ecf1a1ea1190f"),
            (0.0, 1.0, 428.0, 353.0, 2, false, 856, 706, "a505274e85d8194ad6701c3c1cfa4367818170ae956a91d9ab7eaf951bab61e9"),
            (0.0, 1.0, 428.0, 353.0, 2, true, 856, 706, "74cf6d0049f83c5d97fa1b07c5d8ae624f987804606bceb4eb4b181b5795aec6"),
            (0.0, 1.0, 114.0, 94.0, 2, false, 228, 188, "63aa5bb6e228223dfd4b69e5095d2f43ef2a39e82500e5912c17871c58cc5b2a"),
            (0.0, 1.0, 114.0, 94.0, 2, true, 228, 188, "63aa5bb6e228223dfd4b69e5095d2f43ef2a39e82500e5912c17871c58cc5b2a"),
            (0.0, 0.6, 342.0, 282.0, 1, false, 342, 282, "86c23a408193ce19790fe4b93f49bb91727afe089a7c827aa9681117944772f3"),
            (0.0, 0.6, 342.0, 282.0, 1, true, 342, 282, "86c23a408193ce19790fe4b93f49bb91727afe089a7c827aa9681117944772f3"),
            (0.0, 0.6, 428.0, 353.0, 2, false, 856, 706, "2b7a06a99535c1f4f3b28965e4d8e7290194d8f91dae1ec626e8ab43f537fa5d"),
            (0.0, 0.6, 428.0, 353.0, 2, true, 856, 706, "0213fd2c1c751e6a007a97ee7767a44e4bfdf51b50ba26ca6be3bf531a525912"),
            (0.0, 0.6, 114.0, 94.0, 2, false, 228, 188, "0eba6e4a9dbdf0c5d260bb040b5577395acb4ee2aacb384ddb90ef4b97804094"),
            (0.0, 0.6, 114.0, 94.0, 2, true, 228, 188, "0eba6e4a9dbdf0c5d260bb040b5577395acb4ee2aacb384ddb90ef4b97804094"),
            (8.0, 1.0, 342.0, 282.0, 1, false, 342, 282, "8945675a4b996d19c7e0a51837f40e378de1241c4c5d286f7f2c1190111315bf"),
            (8.0, 1.0, 342.0, 282.0, 1, true, 342, 282, "8945675a4b996d19c7e0a51837f40e378de1241c4c5d286f7f2c1190111315bf"),
            (8.0, 1.0, 428.0, 353.0, 2, false, 856, 706, "416cacc5e82f191e59e73824203b129762c4c0f546d39058220923e6decea584"),
            (8.0, 1.0, 428.0, 353.0, 2, true, 856, 706, "03017adf743e9be8e8d0f238669efaa674df3ef368a721eb5838fbc8c22150ed"),
            (8.0, 1.0, 114.0, 94.0, 2, false, 228, 188, "3af84f8d3e538c975439d7368f3f07c8babc26685ecad653f773a3f1abecc157"),
            (8.0, 1.0, 114.0, 94.0, 2, true, 228, 188, "3af84f8d3e538c975439d7368f3f07c8babc26685ecad653f773a3f1abecc157"),
            (8.0, 0.6, 342.0, 282.0, 1, false, 342, 282, "c01063945920d5166e8f5197a0a8a02b480f3b5348d91f83d90bb99884d495a2"),
            (8.0, 0.6, 342.0, 282.0, 1, true, 342, 282, "c01063945920d5166e8f5197a0a8a02b480f3b5348d91f83d90bb99884d495a2"),
            (8.0, 0.6, 428.0, 353.0, 2, false, 856, 706, "aab627ead2cd311395a16fa3e0ee8b80af0c5808cccb769dcfd2aa11f51d8b57"),
            (8.0, 0.6, 428.0, 353.0, 2, true, 856, 706, "60a50d66aedb47ba7482a8cb15c13deaca4039a0c63a55a00841ad0ff43e0015"),
            (8.0, 0.6, 114.0, 94.0, 2, false, 228, 188, "bc8ad08cf0b386f2ce3c087f3d395c841bde9dd45f5b37719f8b0d20c0c64dae"),
            (8.0, 0.6, 114.0, 94.0, 2, true, 228, 188, "bc8ad08cf0b386f2ce3c087f3d395c841bde9dd45f5b37719f8b0d20c0c64dae"),
        ] {
            let align = if center { "xMidYMid" } else { "xMinYMin" };
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{logical_width}" height="{logical_height}" viewBox="0 0 342 282" preserveAspectRatio="{align} meet"><svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="342" height="282" viewBox="-101 -101 342 282"><g opacity="{opacity}" data-d2-rs-dash-owner-id="control-native-raster-shape-0" data-d2-rs-dash-size-bits="4024000000000000" data-d2-rs-dash-gap-bits="4023bb3511f45fb3" data-d2-rs-dash-offset-bits="0000000000000000"><rect id="control-native-raster-shape-0-primitive-0" x="0" y="0" width="140" height="80" rx="{radius}" ry="{radius}" fill="#f7f8fe" stroke="#0d32b2" stroke-width="2" stroke-linejoin="round" stroke-dasharray="10 9.865639267998494"/></g></svg></svg>"##
            );
            let exact = crate::collect_exact_dash_parameters(&svg);
            assert_eq!(exact.len(), 1);
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let tree = usvg::Tree::from_str(&tagged, &usvg::Options::default()).unwrap();
            let (width, height, pixels) =
                super::render_simple_with_sources_and_background_and_dash_parameters(
                    &tree,
                    scale,
                    &[],
                    &sources,
                    [255; 4],
                    &exact,
                )
                .unwrap();
            assert_eq!((width, height), (expected_width, expected_height));
            let digest = format!("{:x}", Sha256::digest(&pixels));
            if digest != expected {
                failures.push(format!("radius{radius} opacity{opacity} {logical_width}x{logical_height} scale{scale} center{center}: native {digest} Source {expected}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    #[rustfmt::skip]
    fn native_closed_dash_wrap_and_full_contour_match_original_go_rgba_controls() {
        use sha2::{Digest, Sha256};
        let mut failures = Vec::new();
        for (dash, gap, offset, radius, opacity, logical_width, logical_height, scale, center, expected_width, expected_height, expected) in [
            (1000.0_f64, 1.0_f64, 0.0_f64, 0.0, 1.0, 342.0, 282.0, 1, false, 342, 282, "4f8ebd5a15f928ce14a16f0c41378893460ee99895e2befaed3a9475dacddc50"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 0.0, 1.0, 342.0, 282.0, 1, true, 342, 282, "4f8ebd5a15f928ce14a16f0c41378893460ee99895e2befaed3a9475dacddc50"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 0.0, 1.0, 428.0, 353.0, 2, false, 856, 706, "bc4dc70dabf327205dc1aebe84031c765da25d28f2cd7ec66fa2ea7f3a727092"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 0.0, 1.0, 428.0, 353.0, 2, true, 856, 706, "dd732b7dbf1676944c784f35e47d6f7ca6f38b54c5c50d08597d6e3f4c0a40c7"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 0.0, 1.0, 114.0, 94.0, 2, false, 228, 188, "4a18021550f33bde441989f41e2134a99450ac99892c5f5b5b339659c899c369"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 0.0, 1.0, 114.0, 94.0, 2, true, 228, 188, "4a18021550f33bde441989f41e2134a99450ac99892c5f5b5b339659c899c369"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 0.0, 0.6, 342.0, 282.0, 1, false, 342, 282, "5393053d1d95a0c02d6f3d0f03fa04878f24a689e06db279bdf6b0c62aaf92ea"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 0.0, 0.6, 342.0, 282.0, 1, true, 342, 282, "5393053d1d95a0c02d6f3d0f03fa04878f24a689e06db279bdf6b0c62aaf92ea"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 0.0, 0.6, 428.0, 353.0, 2, false, 856, 706, "dd8ad83efd24936145a070f4c44ff601002de610b12929a11b588c35101df7a9"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 0.0, 0.6, 428.0, 353.0, 2, true, 856, 706, "16d526684a8d609cff422a99d8733502455270b293e1558d90ed77ecc9072be6"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 0.0, 0.6, 114.0, 94.0, 2, false, 228, 188, "133f18b6a81436d39d9cec86c74b061b5bfa61491ae4b42a05e255b727232447"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 0.0, 0.6, 114.0, 94.0, 2, true, 228, 188, "133f18b6a81436d39d9cec86c74b061b5bfa61491ae4b42a05e255b727232447"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 7.75, 1.0, 342.0, 282.0, 1, false, 342, 282, "0f57e4754b47359e97dc8ceb5524dd0c2c84a1d4f4228e94c46c8aec223236f9"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 7.75, 1.0, 342.0, 282.0, 1, true, 342, 282, "0f57e4754b47359e97dc8ceb5524dd0c2c84a1d4f4228e94c46c8aec223236f9"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 7.75, 1.0, 428.0, 353.0, 2, false, 856, 706, "3ae646aa8bc266c1981f41c60bb39efaef2b53d5b2d1d335d60f13b26e1287d0"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 7.75, 1.0, 428.0, 353.0, 2, true, 856, 706, "8d804724d40c6ba2076924bf50a577ebd58f371f6abe68b4ed306f36ba9619bc"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 7.75, 1.0, 114.0, 94.0, 2, false, 228, 188, "baff0df66d427983ab76bf2d0770a9ef35d6968ecabd86386befccbfe40d66b7"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 7.75, 1.0, 114.0, 94.0, 2, true, 228, 188, "baff0df66d427983ab76bf2d0770a9ef35d6968ecabd86386befccbfe40d66b7"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 7.75, 0.6, 342.0, 282.0, 1, false, 342, 282, "9b9d05e12278887f84bdef4a51e6d188162ffda50fcd886269f1e19e2f3c1e1a"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 7.75, 0.6, 342.0, 282.0, 1, true, 342, 282, "9b9d05e12278887f84bdef4a51e6d188162ffda50fcd886269f1e19e2f3c1e1a"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 7.75, 0.6, 428.0, 353.0, 2, false, 856, 706, "c1488501b546552bf282a2a6de1846be86ec15efac97f229502f30c19f0c7ff0"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 7.75, 0.6, 428.0, 353.0, 2, true, 856, 706, "b476c7cf43ce0cf6f5836a7a9ebfb0298d2d663a7e545be1078d792697bca8cd"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 7.75, 0.6, 114.0, 94.0, 2, false, 228, 188, "0a95f8bbe2d364a9012f7d502a757b69c9ea2d1a383d80fa61f8a19951064eef"),
            (1000.0_f64, 1.0_f64, 0.0_f64, 7.75, 0.6, 114.0, 94.0, 2, true, 228, 188, "0a95f8bbe2d364a9012f7d502a757b69c9ea2d1a383d80fa61f8a19951064eef"),
            (10.0_f64, 7.0_f64, -3.25_f64, 0.0, 1.0, 342.0, 282.0, 1, false, 342, 282, "f6788bedf7e96911567e6c0848ba797f1c9b9add8d7e5ed2e4a913c8711ed71b"),
            (10.0_f64, 7.0_f64, -3.25_f64, 0.0, 1.0, 342.0, 282.0, 1, true, 342, 282, "f6788bedf7e96911567e6c0848ba797f1c9b9add8d7e5ed2e4a913c8711ed71b"),
            (10.0_f64, 7.0_f64, -3.25_f64, 0.0, 1.0, 428.0, 353.0, 2, false, 856, 706, "61b3852ade3b750c407d56ad54d16eb0ee7fdbcd302c8fb974a375e202e2e79f"),
            (10.0_f64, 7.0_f64, -3.25_f64, 0.0, 1.0, 428.0, 353.0, 2, true, 856, 706, "eb802ca9a27785838db23d472a87a4416585f89a70a21b19e1c02e7c4c12cbce"),
            (10.0_f64, 7.0_f64, -3.25_f64, 0.0, 1.0, 114.0, 94.0, 2, false, 228, 188, "cd1e7718e316ebb904d815faaa840a2e7f84fb3096934927438eaba7799f7741"),
            (10.0_f64, 7.0_f64, -3.25_f64, 0.0, 1.0, 114.0, 94.0, 2, true, 228, 188, "cd1e7718e316ebb904d815faaa840a2e7f84fb3096934927438eaba7799f7741"),
            (10.0_f64, 7.0_f64, -3.25_f64, 0.0, 0.6, 342.0, 282.0, 1, false, 342, 282, "7bc43ab24bd22999a8e423a40f14b89510952f50719c7dab4ef6962f5726e777"),
            (10.0_f64, 7.0_f64, -3.25_f64, 0.0, 0.6, 342.0, 282.0, 1, true, 342, 282, "7bc43ab24bd22999a8e423a40f14b89510952f50719c7dab4ef6962f5726e777"),
            (10.0_f64, 7.0_f64, -3.25_f64, 0.0, 0.6, 428.0, 353.0, 2, false, 856, 706, "27c3d602cdf3704158b493d3c970cf3e908c7c633a647bf59d4105379f7c8888"),
            (10.0_f64, 7.0_f64, -3.25_f64, 0.0, 0.6, 428.0, 353.0, 2, true, 856, 706, "45f2b8213b1cc2e4fa0d4d342ba89321c979b6380dafb48d5acb1843982c67ec"),
            (10.0_f64, 7.0_f64, -3.25_f64, 0.0, 0.6, 114.0, 94.0, 2, false, 228, 188, "4de28c6b1aedb4e09b16d9ad963b0c3a1e84b46642170fea43109a7267b97c73"),
            (10.0_f64, 7.0_f64, -3.25_f64, 0.0, 0.6, 114.0, 94.0, 2, true, 228, 188, "4de28c6b1aedb4e09b16d9ad963b0c3a1e84b46642170fea43109a7267b97c73"),
            (10.0_f64, 7.0_f64, -3.25_f64, 7.75, 1.0, 342.0, 282.0, 1, false, 342, 282, "dc6d33e5df204cd7853938cd2657910d4fa2883b5731f5b330ed2ce475376551"),
            (10.0_f64, 7.0_f64, -3.25_f64, 7.75, 1.0, 342.0, 282.0, 1, true, 342, 282, "dc6d33e5df204cd7853938cd2657910d4fa2883b5731f5b330ed2ce475376551"),
            (10.0_f64, 7.0_f64, -3.25_f64, 7.75, 1.0, 428.0, 353.0, 2, false, 856, 706, "f6aefeaa00adcc62e4c10db865d58262a8a059a895bda42fd24e2c867d8423b4"),
            (10.0_f64, 7.0_f64, -3.25_f64, 7.75, 1.0, 428.0, 353.0, 2, true, 856, 706, "70bdc0b92433ba3a33eaaa96f8a23c72bfea0bedebf680fdcce3c2d0a5f5118c"),
            (10.0_f64, 7.0_f64, -3.25_f64, 7.75, 1.0, 114.0, 94.0, 2, false, 228, 188, "33f34470336847d24178623839c4ccc741d67b3a03e3582f93af3c3f587b5086"),
            (10.0_f64, 7.0_f64, -3.25_f64, 7.75, 1.0, 114.0, 94.0, 2, true, 228, 188, "33f34470336847d24178623839c4ccc741d67b3a03e3582f93af3c3f587b5086"),
            (10.0_f64, 7.0_f64, -3.25_f64, 7.75, 0.6, 342.0, 282.0, 1, false, 342, 282, "d44c0c7f42fd8c510dcd5a48ad9014171128f73cc74aac9d3118deb4f1d98121"),
            (10.0_f64, 7.0_f64, -3.25_f64, 7.75, 0.6, 342.0, 282.0, 1, true, 342, 282, "d44c0c7f42fd8c510dcd5a48ad9014171128f73cc74aac9d3118deb4f1d98121"),
            (10.0_f64, 7.0_f64, -3.25_f64, 7.75, 0.6, 428.0, 353.0, 2, false, 856, 706, "070732b7fd881353ee28c5bc75b8f9c74a148c1244287aa48eb63d8ac47bb9c1"),
            (10.0_f64, 7.0_f64, -3.25_f64, 7.75, 0.6, 428.0, 353.0, 2, true, 856, 706, "076916b5187c53cd4b2592328d65d536ab497656e7e819b9b1cd01405e909422"),
            (10.0_f64, 7.0_f64, -3.25_f64, 7.75, 0.6, 114.0, 94.0, 2, false, 228, 188, "38a262c54458f8c996e0b49129248350e8f3610ee493e6fcd95eb2a2168a7842"),
            (10.0_f64, 7.0_f64, -3.25_f64, 7.75, 0.6, 114.0, 94.0, 2, true, 228, 188, "38a262c54458f8c996e0b49129248350e8f3610ee493e6fcd95eb2a2168a7842"),
            (10.0_f64, 7.0_f64, 3.25_f64, 0.0, 1.0, 342.0, 282.0, 1, false, 342, 282, "ae47259861e2611bf6abe4beab85d87bbe8288893b40494a74afde66f4d4da1a"),
            (10.0_f64, 7.0_f64, 3.25_f64, 0.0, 1.0, 342.0, 282.0, 1, true, 342, 282, "ae47259861e2611bf6abe4beab85d87bbe8288893b40494a74afde66f4d4da1a"),
            (10.0_f64, 7.0_f64, 3.25_f64, 0.0, 1.0, 428.0, 353.0, 2, false, 856, 706, "c4e801ac1d4b9ccc55948c83f11479fa42728092ca0cf6636d17d72dee0b555f"),
            (10.0_f64, 7.0_f64, 3.25_f64, 0.0, 1.0, 428.0, 353.0, 2, true, 856, 706, "0ef41ddafc2b15185595fc525150da37db6a5c0e1cdfea472c9751a2ebd5bf5a"),
            (10.0_f64, 7.0_f64, 3.25_f64, 0.0, 1.0, 114.0, 94.0, 2, false, 228, 188, "47d3746afe86c760b291dab407629cd76ef6df7eeb057c5dabe91eb21ee01a4c"),
            (10.0_f64, 7.0_f64, 3.25_f64, 0.0, 1.0, 114.0, 94.0, 2, true, 228, 188, "47d3746afe86c760b291dab407629cd76ef6df7eeb057c5dabe91eb21ee01a4c"),
            (10.0_f64, 7.0_f64, 3.25_f64, 0.0, 0.6, 342.0, 282.0, 1, false, 342, 282, "cfd0c447813e6d52b338c4715bfd44e14a6f379610c9cbe3656d60377324a766"),
            (10.0_f64, 7.0_f64, 3.25_f64, 0.0, 0.6, 342.0, 282.0, 1, true, 342, 282, "cfd0c447813e6d52b338c4715bfd44e14a6f379610c9cbe3656d60377324a766"),
            (10.0_f64, 7.0_f64, 3.25_f64, 0.0, 0.6, 428.0, 353.0, 2, false, 856, 706, "283b817d84d45b2290932294937f08bb6552fa97f88ec18f8061f8d01923aab5"),
            (10.0_f64, 7.0_f64, 3.25_f64, 0.0, 0.6, 428.0, 353.0, 2, true, 856, 706, "2c03062acbca8dcd2d24196c7bfc18d80162012dd78a93cec4ce67ccecabce6c"),
            (10.0_f64, 7.0_f64, 3.25_f64, 0.0, 0.6, 114.0, 94.0, 2, false, 228, 188, "5dd74c730aa1def8d61c7059bf196a7e1b6d474a68e4e30716ffe2a5426b4724"),
            (10.0_f64, 7.0_f64, 3.25_f64, 0.0, 0.6, 114.0, 94.0, 2, true, 228, 188, "5dd74c730aa1def8d61c7059bf196a7e1b6d474a68e4e30716ffe2a5426b4724"),
            (10.0_f64, 7.0_f64, 3.25_f64, 7.75, 1.0, 342.0, 282.0, 1, false, 342, 282, "962e62474d133d49f4749d0253a4926150626029f98205163b38e7f232786146"),
            (10.0_f64, 7.0_f64, 3.25_f64, 7.75, 1.0, 342.0, 282.0, 1, true, 342, 282, "962e62474d133d49f4749d0253a4926150626029f98205163b38e7f232786146"),
            (10.0_f64, 7.0_f64, 3.25_f64, 7.75, 1.0, 428.0, 353.0, 2, false, 856, 706, "f1a749f3eb98f73ac661fec523c847587280818f47c4bd751d701954c0a5442e"),
            (10.0_f64, 7.0_f64, 3.25_f64, 7.75, 1.0, 428.0, 353.0, 2, true, 856, 706, "e02984e0ee536701de7c82069fdbc27aa87fd2a201c5a159a2c1fb65a37c6f56"),
            (10.0_f64, 7.0_f64, 3.25_f64, 7.75, 1.0, 114.0, 94.0, 2, false, 228, 188, "43e28edc2334f818a2b5f29ceca3a3f41c1631472c64cc11a977e7e874e8049c"),
            (10.0_f64, 7.0_f64, 3.25_f64, 7.75, 1.0, 114.0, 94.0, 2, true, 228, 188, "43e28edc2334f818a2b5f29ceca3a3f41c1631472c64cc11a977e7e874e8049c"),
            (10.0_f64, 7.0_f64, 3.25_f64, 7.75, 0.6, 342.0, 282.0, 1, false, 342, 282, "ee18bfd44787d7916274610e7be8a334a4040deda75e250e02bf45484de439d9"),
            (10.0_f64, 7.0_f64, 3.25_f64, 7.75, 0.6, 342.0, 282.0, 1, true, 342, 282, "ee18bfd44787d7916274610e7be8a334a4040deda75e250e02bf45484de439d9"),
            (10.0_f64, 7.0_f64, 3.25_f64, 7.75, 0.6, 428.0, 353.0, 2, false, 856, 706, "ebe0c48da5af5a1c642837ab754b969712868d5b31357a113c4d23e605f92b34"),
            (10.0_f64, 7.0_f64, 3.25_f64, 7.75, 0.6, 428.0, 353.0, 2, true, 856, 706, "5bc8b955eeb6f740ba2f2d856c79145bc3001154d246d881f46b09fd08d99302"),
            (10.0_f64, 7.0_f64, 3.25_f64, 7.75, 0.6, 114.0, 94.0, 2, false, 228, 188, "68768da5f4ddbea84c314332f473ad58aaf74343c12478df3e0043c5dafa871a"),
            (10.0_f64, 7.0_f64, 3.25_f64, 7.75, 0.6, 114.0, 94.0, 2, true, 228, 188, "68768da5f4ddbea84c314332f473ad58aaf74343c12478df3e0043c5dafa871a"),
        ] {
            let align = if center { "xMidYMid" } else { "xMinYMin" };
            let (dash_bits, gap_bits, offset_bits) = (dash.to_bits(), gap.to_bits(), offset.to_bits());
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{logical_width}" height="{logical_height}" viewBox="0 0 342 282" preserveAspectRatio="{align} meet"><svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="342" height="282" viewBox="-101 -101 342 282"><g opacity="{opacity}" data-d2-rs-dash-owner-id="control-native-raster-shape-0" data-d2-rs-dash-size-bits="{dash_bits:016x}" data-d2-rs-dash-gap-bits="{gap_bits:016x}" data-d2-rs-dash-offset-bits="{offset_bits:016x}"><rect id="control-native-raster-shape-0-primitive-0" x="4.25" y="7.125" width="140.5" height="80.75" rx="{radius}" ry="{radius}" fill="#f7f8fe" stroke="#0d32b2" stroke-width="2" stroke-linejoin="round" stroke-dasharray="{dash} {gap}" stroke-dashoffset="{offset}"/></g></svg></svg>"##
            );
            let exact = crate::collect_exact_dash_parameters(&svg);
            assert_eq!(exact.len(), 1);
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let tree = usvg::Tree::from_str(&tagged, &usvg::Options::default()).unwrap();
            let (width, height, pixels) =
                super::render_simple_with_sources_and_background_and_dash_parameters(
                    &tree,
                    scale,
                    &[],
                    &sources,
                    [255; 4],
                    &exact,
                )
                .unwrap();
            assert_eq!((width, height), (expected_width, expected_height));
            let digest = format!("{:x}", Sha256::digest(&pixels));
            if digest != expected {
                failures.push(format!("dash{dash} gap{gap} offset{offset} radius{radius} opacity{opacity} {logical_width}x{logical_height} scale{scale} center{center}: native {digest} Source {expected}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn native_sharp_shape_shadow_matches_original_go_whole_rgba_controls() {
        use sha2::{Digest, Sha256};
        for (
            logical_width,
            logical_height,
            scale,
            center,
            radius,
            opacity,
            expected_width,
            expected_height,
            expected,
        ) in [
            (
                342.0,
                282.0,
                1,
                false,
                0.0,
                1.0,
                342,
                282,
                "cd76cf23eae7577725c9f8632c6b2fe354020552330f43f1cb80e5c5ff3ea417",
            ),
            (
                342.0,
                282.0,
                1,
                true,
                0.0,
                1.0,
                342,
                282,
                "cd76cf23eae7577725c9f8632c6b2fe354020552330f43f1cb80e5c5ff3ea417",
            ),
            (
                428.0,
                353.0,
                2,
                false,
                0.0,
                1.0,
                856,
                706,
                "3727f59d74605a1db609ef78c031c9b6a7664d2c2bd36c3bd33f62a18d4395be",
            ),
            (
                428.0,
                353.0,
                2,
                true,
                0.0,
                1.0,
                856,
                706,
                "97bb104febe754d722ceac86b00c20841318755647040c81d6364a61c43a5122",
            ),
            (
                114.0,
                94.0,
                2,
                false,
                0.0,
                1.0,
                228,
                188,
                "dd731f151e88eb8cc743576db891ba2021943b496abba80803341d364b1e8db1",
            ),
            (
                114.0,
                94.0,
                2,
                true,
                0.0,
                1.0,
                228,
                188,
                "dd731f151e88eb8cc743576db891ba2021943b496abba80803341d364b1e8db1",
            ),
            (
                342.0,
                282.0,
                1,
                false,
                0.0,
                0.6,
                342,
                282,
                "1c7c20de4166d4837e2d5789db02f7c8355b323fb87002384531c9dad5599e0c",
            ),
            (
                342.0,
                282.0,
                1,
                true,
                0.0,
                0.6,
                342,
                282,
                "1c7c20de4166d4837e2d5789db02f7c8355b323fb87002384531c9dad5599e0c",
            ),
            (
                428.0,
                353.0,
                2,
                false,
                0.0,
                0.6,
                856,
                706,
                "14fbd52968808f4f1df9821b690a810ded9f390a5da53a076436025e96e3dd58",
            ),
            (
                428.0,
                353.0,
                2,
                true,
                0.0,
                0.6,
                856,
                706,
                "69714a1160a372ca23a9784f0452c4ab2e67147cc68bf50496193d918a5f5b71",
            ),
            (
                114.0,
                94.0,
                2,
                false,
                0.0,
                0.6,
                228,
                188,
                "9552fb5cbb43026334dba2800112166d805c7117e912a85f1b89977da1b49404",
            ),
            (
                114.0,
                94.0,
                2,
                true,
                0.0,
                0.6,
                228,
                188,
                "9552fb5cbb43026334dba2800112166d805c7117e912a85f1b89977da1b49404",
            ),
            (
                342.0,
                282.0,
                1,
                false,
                7.75,
                1.0,
                342,
                282,
                "6449edf8b0b3b0bb4a68457b9beb29f5e894c6f26da5d221174890e05d677355",
            ),
            (
                342.0,
                282.0,
                1,
                true,
                7.75,
                1.0,
                342,
                282,
                "6449edf8b0b3b0bb4a68457b9beb29f5e894c6f26da5d221174890e05d677355",
            ),
            (
                428.0,
                353.0,
                2,
                false,
                7.75,
                1.0,
                856,
                706,
                "0c22d28389794d8d74ed4c8a9e695ae57ba2596d94f2584ce356c0c068b59b2e",
            ),
            (
                428.0,
                353.0,
                2,
                true,
                7.75,
                1.0,
                856,
                706,
                "a06be1f6c7e25bf4940daf3f330741cb7cf0071587b56f77f803eb7eb55ad708",
            ),
            (
                114.0,
                94.0,
                2,
                false,
                7.75,
                1.0,
                228,
                188,
                "eb47148287d48b59c89531395d9f4c4f937b0020f79797d379cc872e210dae23",
            ),
            (
                114.0,
                94.0,
                2,
                true,
                7.75,
                1.0,
                228,
                188,
                "eb47148287d48b59c89531395d9f4c4f937b0020f79797d379cc872e210dae23",
            ),
            (
                342.0,
                282.0,
                1,
                false,
                7.75,
                0.6,
                342,
                282,
                "2fb5a82cec72e95307571221a3dc0be63257bce0f99d43616485e3b49c4ddbb7",
            ),
            (
                342.0,
                282.0,
                1,
                true,
                7.75,
                0.6,
                342,
                282,
                "2fb5a82cec72e95307571221a3dc0be63257bce0f99d43616485e3b49c4ddbb7",
            ),
            (
                428.0,
                353.0,
                2,
                false,
                7.75,
                0.6,
                856,
                706,
                "b747444facc22c375bccbfae4c76ae5670468d96cc77dec42730dc048b55b642",
            ),
            (
                428.0,
                353.0,
                2,
                true,
                7.75,
                0.6,
                856,
                706,
                "fc97833792d22cced5ebb4bc37c1264d2961618080e4a2bddb14de04e7769804",
            ),
            (
                114.0,
                94.0,
                2,
                false,
                7.75,
                0.6,
                228,
                188,
                "61c59008d7bce8328b64638ac3edb37eb51649a78ce91cdf4c25a14f1885218a",
            ),
            (
                114.0,
                94.0,
                2,
                true,
                7.75,
                0.6,
                228,
                188,
                "61c59008d7bce8328b64638ac3edb37eb51649a78ce91cdf4c25a14f1885218a",
            ),
        ] {
            let align = if center { "xMidYMid" } else { "xMinYMin" };
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{logical_width}" height="{logical_height}" viewBox="0 0 342 282" preserveAspectRatio="{align} meet"><defs><filter id="shadow" x="-20%" y="-20%" width="160%" height="160%"><feFlood flood-color="#3d4574" flood-opacity="0.4" result="shadow-color"/><feComposite in="shadow-color" in2="SourceAlpha" operator="in"/><feOffset dx="3" dy="5"/><feBlend in="SourceGraphic" mode="normal"/></filter></defs><svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="342" height="282" viewBox="-101 -101 342 282"><g opacity="{opacity}"><g id="d2-rs-native-shape-shadow-control" data-d2-rs-shape-shadow="sharp" filter="url(#shadow)"><rect x="4.25" y="7.125" width="140.5" height="80.75" rx="{radius}" ry="{radius}" fill="#f7f8fe" stroke="#0d32b2" stroke-width="2" stroke-linejoin="round"/></g></g></svg></svg>"##
            );
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            assert!(sources.is_native_shape_shadow("d2-rs-native-shape-shadow-control"));
            let tree = usvg::Tree::from_str(&tagged, &usvg::Options::default()).unwrap();
            let (width, height, pixels) =
                super::render_simple_with_sources(&tree, scale, &[], &sources).unwrap();
            assert_eq!((width, height), (expected_width, expected_height));
            assert_eq!(
                format!("{:x}", Sha256::digest(&pixels)),
                expected,
                "Source sharp shadow {logical_width}x{logical_height} scale{scale} center{center} radius{radius} opacity{opacity}"
            );
            // A blended first geometry child has a transparent backdrop:
            // Source's blend equation preserves this complete control frame.
            // Exercise crop translation and require the native backend itself.
            for mode in ["multiply", "darken", "color-burn", "overlay", "lighten"] {
                let blended = svg
                    .replace(
                        "<rect x=",
                        &format!("<g style=\"mix-blend-mode:{mode}\"><rect x="),
                    )
                    .replace("/></g></g></svg>", "/></g></g></g></svg>");
                let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&blended);
                let tree = usvg::Tree::from_str(&tagged, &usvg::Options::default()).unwrap();
                let (width, height, pixels) =
                    super::render_simple_with_sources(&tree, scale, &[], &sources)
                        .expect("native blended shadow geometry");
                assert_eq!((width, height), (expected_width, expected_height));
                assert_eq!(
                    format!("{:x}", Sha256::digest(&pixels)),
                    expected,
                    "Source transparent-backdrop shadow geometry {mode}"
                );
            }
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn native_underline_matches_original_go_whole_rgba_controls() {
        use sha2::{Digest, Sha256};
        let oracle: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/native-underline-go-v090.json"
        ))
        .unwrap();
        for row in oracle["controls"].as_array().unwrap() {
            let face = row["face"].as_u64().unwrap();
            let logical_width = row["logical_width"].as_f64().unwrap();
            let logical_height = row["logical_height"].as_f64().unwrap();
            let scale = row["scale"].as_u64().unwrap() as u32;
            let family = if face >= 2 {
                "Source Code Pro"
            } else {
                "Source Sans Pro"
            };
            let weight = if face % 2 == 1 { "bold" } else { "normal" };
            let align = if row["center"].as_bool().unwrap() {
                "xMidYMid"
            } else {
                "xMinYMin"
            };
            let x_bits = format!("{:016x}", 70.25_f64.to_bits());
            let y_bits = format!("{:016x}", 46.5_f64.to_bits());
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{logical_width}" height="{logical_height}" viewBox="0 0 342 282" preserveAspectRatio="{align} meet"><svg id="d2-rs-native-canvas-viewport-underline" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="342" height="282" viewBox="-101 -101 342 282"><text id="label" x="70.25" y="46.5" data-d2-rs-text-x-bits="{x_bits}" data-d2-rs-text-y-bits="{y_bits}" text-anchor="middle" font-family="{family}" font-size="16" font-weight="{weight}" text-decoration="underline" xml:space="preserve" fill="#0a0f25">gy p  </text></svg></svg>"##
            );
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let mut options = usvg::Options::default();
            super::super::load_bundled_fonts(&mut options);
            let tree = usvg::Tree::from_str(&tagged, &options).unwrap();
            let (width, height, pixels) =
                super::render_simple_with_sources(&tree, scale, &[], &sources).unwrap();
            assert_eq!(
                (u64::from(width), u64::from(height)),
                (
                    row["width"].as_u64().unwrap(),
                    row["height"].as_u64().unwrap()
                )
            );
            assert_eq!(
                format!("{:x}", Sha256::digest(&pixels)),
                row["rgba_sha256"].as_str().unwrap(),
                "Source underline face{face} {logical_width}x{logical_height} scale{scale} {align}"
            );
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn translated_opacity_text_bounds_match_original_go_whole_rgba_controls() {
        let oracle = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/effect-text-bounds-go-v090.json"
        ))
        .unwrap();
        check_translated_opacity_text_bounds_source_controls(&oracle);
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn overhanging_opacity_text_matches_original_go_whole_rgba_controls() {
        let oracle: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/effect-text-overhang-go-v090.json"
        ))
        .unwrap();
        assert_eq!(oracle["controls"].as_array().unwrap().len(), 72);
        check_translated_opacity_text_bounds_source_controls(&oracle);
    }

    #[cfg(feature = "oracle-fixtures")]
    fn check_translated_opacity_text_bounds_source_controls(oracle: &serde_json::Value) {
        use sha2::{Digest, Sha256};
        let mut failures = Vec::new();
        for row in oracle["controls"].as_array().unwrap() {
            let face = row["face"].as_u64().unwrap();
            let logical_width = row["logical_width"].as_f64().unwrap();
            let logical_height = row["logical_height"].as_f64().unwrap();
            let scale = row["scale"].as_u64().unwrap() as u32;
            let family = if face >= 2 {
                "Source Code Pro"
            } else {
                "Source Sans Pro"
            };
            let weight = if face % 2 == 1 { "bold" } else { "normal" };
            let align = if row["center"].as_bool().unwrap() {
                "xMidYMid"
            } else {
                "xMinYMin"
            };
            let origin_x = row["origin_x"].as_f64().unwrap_or(70.25);
            let origin_y = row["origin_y"].as_f64().unwrap_or(46.5);
            let x_bits = format!("{:016x}", origin_x.to_bits());
            let y_bits = format!("{:016x}", origin_y.to_bits());
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{logical_width}" height="{logical_height}" viewBox="0 0 342 282" preserveAspectRatio="{align} meet"><svg id="d2-rs-native-canvas-viewport-underline" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="342" height="282" viewBox="-101 -101 342 282"><g id="effect-transform" opacity="0.6" transform="translate(13.125 17.25)"><rect x="4.25" y="7.125" width="140.5" height="80.75" fill="#f7f8fe" stroke="#0d32b2" stroke-width="2" stroke-linejoin="round"/><text id="label" x="{origin_x}" y="{origin_y}" data-d2-rs-text-x-bits="{x_bits}" data-d2-rs-text-y-bits="{y_bits}" text-anchor="middle" font-family="{family}" font-size="16" font-weight="{weight}" text-decoration="underline" xml:space="preserve" fill="#0a0f25">gy p  </text></g></svg></svg>"##
            );
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let mut options = usvg::Options::default();
            super::super::load_bundled_fonts(&mut options);
            let tree = usvg::Tree::from_str(&tagged, &options).unwrap();
            let (width, height, pixels) =
                super::render_simple_with_sources(&tree, scale, &[], &sources).unwrap();
            assert_eq!(
                (u64::from(width), u64::from(height)),
                (
                    row["width"].as_u64().unwrap(),
                    row["height"].as_u64().unwrap()
                )
            );
            let actual = format!("{:x}", Sha256::digest(&pixels));
            #[cfg(feature = "diagnostic-traces")]
            if let Some(directory) = std::env::var_os("D2_RS_TEST_PIXEL_DUMP") {
                let path = std::path::PathBuf::from(directory).join(format!(
                    "native-{face}-{}.png",
                    row["case"].as_u64().unwrap()
                ));
                let file = std::fs::File::create(path).unwrap();
                let mut encoder = png::Encoder::new(file, width, height);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()
                    .unwrap()
                    .write_image_data(&pixels)
                    .unwrap();
            }
            if actual != row["rgba_sha256"].as_str().unwrap() {
                failures.push(format!("face{face} {logical_width}x{logical_height} scale{scale} {align}: native{actual} Source{}", row["rgba_sha256"].as_str().unwrap()));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn native_viewport_text_matches_original_go_whole_rgba_controls() {
        use sha2::{Digest, Sha256};
        for (
            face,
            logical_width,
            logical_height,
            scale,
            center,
            expected_width,
            expected_height,
            expected,
        ) in [
            (
                0,
                428.0,
                353.0,
                2,
                false,
                856,
                706,
                "71bce8d3bd2369e287301ea62f73e1d8e021c784d319b3210656fa064d242419",
            ),
            (
                0,
                428.0,
                353.0,
                2,
                true,
                856,
                706,
                "fe99f0d7e4914ed9e02c9cfb9dc46b0573e333bc3e6088d9efdb9b7f6d6c7998",
            ),
            (
                0,
                342.0,
                282.0,
                2,
                false,
                684,
                564,
                "b1ef66bae9eba0b87afdd5ddd090be582050d72201624cd36a601c0f7a0935a5",
            ),
            (
                0,
                342.0,
                282.0,
                2,
                true,
                684,
                564,
                "b1ef66bae9eba0b87afdd5ddd090be582050d72201624cd36a601c0f7a0935a5",
            ),
            (
                0,
                114.0,
                94.0,
                2,
                false,
                228,
                188,
                "d1593263cbcc474708f71a0a320b6df0a74a8a827d3446da473619eaffa5b1f1",
            ),
            (
                0,
                114.0,
                94.0,
                2,
                true,
                228,
                188,
                "d1593263cbcc474708f71a0a320b6df0a74a8a827d3446da473619eaffa5b1f1",
            ),
            (
                1,
                428.0,
                353.0,
                2,
                false,
                856,
                706,
                "6dcacb0bc871267382d2e4ba2bbc0c32fdf26db77c41cb0b2c6241c3aca66eab",
            ),
            (
                1,
                428.0,
                353.0,
                2,
                true,
                856,
                706,
                "76283e84090b411dbd1b5fff461d805ac4c660812f01e197230d71ea7e340a89",
            ),
            (
                1,
                342.0,
                282.0,
                2,
                false,
                684,
                564,
                "f4835c97ac6a786e937d2afa1d3bf4163217e0150cd46afc0b6116e037ca8ef3",
            ),
            (
                1,
                342.0,
                282.0,
                2,
                true,
                684,
                564,
                "f4835c97ac6a786e937d2afa1d3bf4163217e0150cd46afc0b6116e037ca8ef3",
            ),
            (
                1,
                114.0,
                94.0,
                2,
                false,
                228,
                188,
                "9e00fb87e3edb464442626c516a37bcdaa1bd9713bfcf3704711f892326301b2",
            ),
            (
                1,
                114.0,
                94.0,
                2,
                true,
                228,
                188,
                "9e00fb87e3edb464442626c516a37bcdaa1bd9713bfcf3704711f892326301b2",
            ),
            (
                2,
                428.0,
                353.0,
                2,
                false,
                856,
                706,
                "7c5074230e21b908cbe736241d3d1733d6850ab9f55833cc7cbae7f0117f63f9",
            ),
            (
                2,
                428.0,
                353.0,
                2,
                true,
                856,
                706,
                "88a0771c783ccfbebef1ec0bb32bdb5f70b6acb7ad5b7d9c2bd89f1a9d51ca5c",
            ),
            (
                2,
                342.0,
                282.0,
                2,
                false,
                684,
                564,
                "7222dc575790154f2cc83ac8676607fe150579096e3c84b43199730013972b15",
            ),
            (
                2,
                342.0,
                282.0,
                2,
                true,
                684,
                564,
                "7222dc575790154f2cc83ac8676607fe150579096e3c84b43199730013972b15",
            ),
            (
                2,
                114.0,
                94.0,
                2,
                false,
                228,
                188,
                "2516c3079f44dce35aa8b2d3e5ae0dfca3301e99c52537d2fc5eea1d1126d853",
            ),
            (
                2,
                114.0,
                94.0,
                2,
                true,
                228,
                188,
                "2516c3079f44dce35aa8b2d3e5ae0dfca3301e99c52537d2fc5eea1d1126d853",
            ),
            (
                3,
                428.0,
                353.0,
                2,
                false,
                856,
                706,
                "7014713c351b5aa0a3fdd6528a7ae81a1ba8f9a419af5d04878f4bb0d3058ac5",
            ),
            (
                3,
                428.0,
                353.0,
                2,
                true,
                856,
                706,
                "82b72a24a5112ec7923e3d258f6d3b568de570879e28e412b08e54d4bc64e364",
            ),
            (
                3,
                342.0,
                282.0,
                2,
                false,
                684,
                564,
                "c685bc1b307526d8ed2dc2fe13b8c7e701fb95714804da97d999b8bfd5e59b72",
            ),
            (
                3,
                342.0,
                282.0,
                2,
                true,
                684,
                564,
                "c685bc1b307526d8ed2dc2fe13b8c7e701fb95714804da97d999b8bfd5e59b72",
            ),
            (
                3,
                114.0,
                94.0,
                2,
                false,
                228,
                188,
                "330985bdf122f3a98ccbb5dd3260aa4b85a02f2e7862eb0798d1f6be73067558",
            ),
            (
                3,
                114.0,
                94.0,
                2,
                true,
                228,
                188,
                "330985bdf122f3a98ccbb5dd3260aa4b85a02f2e7862eb0798d1f6be73067558",
            ),
        ] {
            let family = if face >= 2 {
                "Source Code Pro"
            } else {
                "Source Sans Pro"
            };
            let weight = if face % 2 == 1 { "bold" } else { "normal" };
            let align = if center { "xMidYMid" } else { "xMinYMin" };
            let x_bits = format!("{:016x}", 70.25_f64.to_bits());
            let y_bits = format!("{:016x}", 46.5_f64.to_bits());
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{logical_width}" height="{logical_height}" viewBox="0 0 342 282" preserveAspectRatio="{align} meet"><svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="342" height="282" viewBox="-101 -101 342 282"><text id="label" x="70.25" y="46.5" data-d2-rs-text-x-bits="{x_bits}" data-d2-rs-text-y-bits="{y_bits}" text-anchor="middle" font-family="{family}" font-size="16" font-weight="{weight}" fill="#0a0f25">matching</text></svg></svg>"##
            );
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let mut options = usvg::Options::default();
            super::super::load_bundled_fonts(&mut options);
            let tree = usvg::Tree::from_str(&tagged, &options).unwrap();
            let (width, height, pixels) =
                super::render_simple_with_sources(&tree, scale, &[], &sources).unwrap();
            assert_eq!((width, height), (expected_width, expected_height));
            assert_eq!(
                format!("{:x}", Sha256::digest(&pixels)),
                expected,
                "Source text face{face} {logical_width}x{logical_height} scale{scale} center{center}"
            );
        }
    }

    #[test]
    fn native_viewport_paths_match_original_go_full_precision_meet_controls() {
        use sha2::{Digest, Sha256};
        for (
            round_join,
            logical_width,
            logical_height,
            scale,
            center,
            expected_width,
            expected_height,
            expected,
        ) in [
            (
                false,
                101.0,
                94.0,
                2,
                false,
                202,
                188,
                "9fc13cd99b2098f540111ee30081211b0a461343a89ffa5a3d8fc8fc339f77f0",
            ),
            (
                false,
                101.0,
                94.0,
                2,
                true,
                202,
                188,
                "1be1ef7b5052d265f96cff1015db551a60c93125652e89493922317c4b26ade8",
            ),
            (
                false,
                378.0,
                353.0,
                2,
                false,
                756,
                706,
                "27cfd8c3d7b041ebdbd3a0db35c7f670a3fc9139846eb8f82dd4176e01bb5f31",
            ),
            (
                false,
                378.0,
                353.0,
                2,
                true,
                756,
                706,
                "aa884867b58ef16b0501e4a5efea7504bfb9addbf358304d71ee21631fbe41c9",
            ),
            (
                false,
                302.0,
                282.0,
                1,
                false,
                302,
                282,
                "8667cd4547961b28f02041eaae1a3556558f931b83e900a6907878b4a9ae563e",
            ),
            (
                false,
                151.0,
                141.0,
                3,
                false,
                453,
                423,
                "ec6f1adb1098d691433adaef0640b98ce4447b09f392d11e490855c63f2045b8",
            ),
            (
                false,
                151.0,
                141.0,
                3,
                true,
                453,
                423,
                "ec6f1adb1098d691433adaef0640b98ce4447b09f392d11e490855c63f2045b8",
            ),
            (
                true,
                101.0,
                94.0,
                2,
                false,
                202,
                188,
                "31f3496e9cc8735d793e54e173863e657f3bf3db0c86cbc7f1fcc2597b6bcd66",
            ),
            (
                true,
                101.0,
                94.0,
                2,
                true,
                202,
                188,
                "b5d3934030f95871378191be4a98847012897a9634a29e317d3a50263040008e",
            ),
            (
                true,
                378.0,
                353.0,
                2,
                false,
                756,
                706,
                "2324809abd93de32c8987fd18630b3cf43e338f094b34e5b7ca633a9c04b925d",
            ),
            (
                true,
                378.0,
                353.0,
                2,
                true,
                756,
                706,
                "3525afe6071d8dc24f3b17c75fb922b5a70954ff98cd64856618be62f733d823",
            ),
            (
                true,
                302.0,
                282.0,
                1,
                false,
                302,
                282,
                "427b2b219ef1af781c527391ebf60c5825967da1f271499f0b619654421357c2",
            ),
            (
                true,
                151.0,
                141.0,
                3,
                false,
                453,
                423,
                "58bc738aacdf989972c8c299daa93a1c5b9b5ac3c0325f273ddd2e3d2d249252",
            ),
            (
                true,
                151.0,
                141.0,
                3,
                true,
                453,
                423,
                "58bc738aacdf989972c8c299daa93a1c5b9b5ac3c0325f273ddd2e3d2d249252",
            ),
        ] {
            let fill = if round_join { "#f7f8fe" } else { "#dee1eb" };
            let join = if round_join { "round" } else { "miter" };
            let align = if center { "xMidYMid" } else { "xMinYMin" };
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{logical_width}" height="{logical_height}" viewBox="0 0 302 282" preserveAspectRatio="{align} meet"><svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="302" height="282" viewBox="-101 -101 302 282"><rect width="100" height="80" fill="{fill}" stroke="#0d32b2" stroke-width="2" stroke-linejoin="{join}" stroke-miterlimit="4"/></svg></svg>"##
            );
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let tree = usvg::Tree::from_str(&tagged, &usvg::Options::default()).unwrap();
            let (width, height, pixels) =
                super::render_simple_with_sources(&tree, scale, &[], &sources).unwrap();
            assert_eq!((width, height), (expected_width, expected_height));
            assert_eq!(
                format!("{:x}", Sha256::digest(&pixels)),
                expected,
                "Source meet {logical_width}x{logical_height} scale{scale} center{center}"
            );
        }
    }

    #[test]
    fn fractional_canvas_dimensions_match_original_go_whole_rgba_frames() {
        use sha2::{Digest, Sha256};
        // Original Go scales float64 logical dimensions before ceil, including
        // boundaries invisible to float32 and canvases smaller than one unit.
        for (x, y, view_width, view_height, scale, expected_width, expected_height, expected) in [
            (
                0.0,
                0.0,
                300.0,
                120.0,
                1,
                300,
                120,
                "a867d3d4bcd9a098692b72def6191ac19e847a5b58bde9d093426ed2584ab33b",
            ),
            (
                0.0,
                0.0,
                300.125,
                120.25,
                1,
                301,
                121,
                "5200917475ea9f5f511ff054abdcdcef5a2f3230d3fcd53ceb5353f55e40d1b6",
            ),
            (
                101.0,
                141.0,
                401.25,
                261.75,
                3,
                1204,
                786,
                "13bf7eb9595e36f98929e5533d8bed95db7eda7e4e429cf08cb16f78004b921f",
            ),
            (
                1001.25,
                777.5,
                1301.25,
                897.5,
                2,
                2603,
                1795,
                "054e32d211271268775d88de91cd39efacc48d82bbb8c8529b2753d2ea399f85",
            ),
            (
                0.0,
                0.0,
                300.0000001,
                120.0000001,
                2,
                601,
                241,
                "36816018374191b9c71d45d254483430a573a89423b2c1fca630ba732cd904e9",
            ),
            (
                0.0,
                0.0,
                0.125,
                0.25,
                3,
                1,
                1,
                "ad95131bc0b799c0b1af477fb14fcf26a6a9f76079e48bf090acb7e8367bfd0e",
            ),
        ] {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{view_width}" height="{view_height}"><g id="effect" opacity="0.7" data-d2-rs-exact-opacity="0.7" transform="translate({x} {y})"><path d="M24 0L225 0C249 0 249 39 249 43C249 47 249 86 225 86L24 86C0 86 0 47 0 43C0 39 0 0 24 0Z" fill="#dee1eb" stroke="#0d32b2" stroke-width="2" stroke-linejoin="round"/></g></svg>"##
            );
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let tree = usvg::Tree::from_str(&tagged, &usvg::Options::default()).unwrap();
            let (width, height, pixels) =
                super::render_simple_with_sources(&tree, scale, &[], &sources).unwrap();
            assert_eq!(
                (width, height),
                (expected_width, expected_height),
                "logical {view_width}x{view_height} scale{scale}"
            );
            assert_eq!(
                format!("{:x}", Sha256::digest(&pixels)),
                expected,
                "Source fractional canvas {view_width}x{view_height} scale{scale}"
            );
        }
    }

    #[test]
    fn curved_opacity_layers_match_original_go_at_clipped_and_translated_origins() {
        use sha2::{Digest, Sha256};
        // Whole RGBA frames from original Go d2scene/d2raster, including
        // a layer clipped at the canvas origin and fractional translation.
        for (variant, x, y, expected) in [
            (
                0,
                0.0,
                0.0,
                "a0e64759fae3953bdaa5c3ea6ebc01353d1094ad881a2ad1dc8f815ef7c6a3b1",
            ),
            (
                0,
                101.0,
                141.0,
                "1ce9abd505b58e1bcf801e47841bfefd77a5c16961cd970e4a66582a4bc6cb65",
            ),
            (
                0,
                1001.25,
                777.5,
                "e054fd5d9bfa348368b4396d4a04baf0a027f613ed69a0d63765c78c9437f282",
            ),
            (
                1,
                0.0,
                0.0,
                "fe02ba28edadc0225473599353654354e5232cfdd7bdfab20bb00acf83d963ff",
            ),
            (
                1,
                101.0,
                141.0,
                "a915431b05f6ab2ebed2dcfe399a5c90dbca45230ae7663da219d71095490f2c",
            ),
            (
                1,
                1001.25,
                777.5,
                "0e7f483609cb6933df6c1e39af2c4853bcee95b5d8c3a05c9edd2b4aebc693b6",
            ),
            (
                2,
                0.0,
                0.0,
                "1cec9388873aab4712b5e25df134b0888eaf0788b744b2b058e911fd59cc5489",
            ),
            (
                2,
                101.0,
                141.0,
                "eaae9cd66f73481c3432c6eaf747679925986905c676ee8c1d9fb2abe168f4fd",
            ),
            (
                2,
                1001.25,
                777.5,
                "4b88735579c5cb9620859b771dd1884ddb07fedecea624a98caecb3350098bf1",
            ),
        ] {
            let view_width = (x + 300.0_f64).ceil();
            let view_height = (y + 120.0_f64).ceil();
            let inner_start = match variant {
                0 => "",
                1 => r#"<g transform="translate(13.125 17.25)">"#,
                _ => {
                    r#"<g id="inner-effect" opacity="0.5" data-d2-rs-exact-opacity="0.5" transform="translate(13.125 17.25)">"#
                }
            };
            let inner_end = if variant == 0 { "" } else { "</g>" };
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{view_width}" height="{view_height}"><g id="effect" opacity="0.7" data-d2-rs-exact-opacity="0.7" transform="translate({x} {y})">{inner_start}<path d="M24 0L225 0C249 0 249 39 249 43C249 47 249 86 225 86L24 86C0 86 0 47 0 43C0 39 0 0 24 0Z" fill="#dee1eb" stroke="#0d32b2" stroke-width="2" stroke-linejoin="round"/>{inner_end}</g></svg>"##
            );
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let tree = usvg::Tree::from_str(&tagged, &usvg::Options::default()).unwrap();
            let (_, _, pixels) =
                super::render_simple_with_sources(&tree, 2, &[], &sources).unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(&pixels)),
                expected,
                "Source curve variant {variant} at {x},{y}"
            );
        }
    }

    #[test]
    fn png_image_scene_placement_matches_original_go_full_pixel_controls() {
        use base64::Engine as _;
        use sha2::{Digest, Sha256};
        let mut rgba = Vec::new();
        for y in 0..349_u32 {
            for x in 0..466_u32 {
                rgba.extend_from_slice(&[x as u8, y as u8, (x + y) as u8, (x * 17 + y * 23) as u8]);
            }
        }
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 466, 349);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&rgba).unwrap();
        }
        let png = base64::engine::general_purpose::STANDARD.encode(encoded);
        // Actual original Go procedural 466x349 asset; every resulting pixel.
        for (view_width, view_height, scale, matrix, x, y, width, height, expected) in [
            (
                300,
                300,
                2,
                "1 0 0 1 0 0",
                0.0,
                0.0,
                300.0,
                300.0,
                "3dec10547d1536b75049313d9c6c67915389638fc64a7794a304d4a0d6ba4d54",
            ),
            (
                300,
                300,
                2,
                "0 1 -1 0 300 0",
                0.0,
                0.0,
                300.0,
                300.0,
                "23a0662181561a72acb43dbc69bb39cc38ea2bf00215498622aef6c0c2b80b2c",
            ),
            (
                300,
                300,
                2,
                "-1 0 0 1 300 0",
                0.0,
                0.0,
                300.0,
                300.0,
                "ec030b72c70566d44b3c282bda7dfbe04c1c70775df7ade5e62d5a85ed183a21",
            ),
            (
                301,
                301,
                2,
                "1 0 0 1 0.125 0.25",
                0.0,
                0.0,
                300.0,
                300.0,
                "47cadc36bf3b5b2b8d8a166511d2e8e158729e9a2a9e57965746ebb5c1fd9940",
            ),
            (
                350,
                350,
                1,
                "1.1 0.05 -0.03 0.95 2 1",
                0.25,
                0.5,
                300.3333333333333,
                299.6666666666667,
                "ae7689204bc99a36082d76710b65bd68ca9198a6d6111e50022fae60adc20709",
            ),
        ] {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="{view_width}" height="{view_height}"><g transform="matrix({matrix})"><image id="scene-image" data-d2-rs-scene-image="ordinary" x="{x}" y="{y}" width="{width}" height="{height}" preserveAspectRatio="none" href="data:image/png;base64,{png}"/></g></svg>"#
            );
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let tree = usvg::Tree::from_str(&tagged, &usvg::Options::default()).unwrap();
            let (_, _, pixels) =
                super::render_simple_with_sources(&tree, scale, &[], &sources).unwrap();
            let digest = format!("{:x}", Sha256::digest(&pixels));
            assert_eq!(digest, expected, "Source scene matrix {matrix}");
        }
    }

    #[test]
    fn nested_pattern_tiles_are_prepared_before_their_consumers() {
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
          <defs>
            <pattern id="outer" width="2" height="2" patternUnits="userSpaceOnUse">
              <rect width="2" height="2" fill="url(#inner)"/>
            </pattern>
            <pattern id="inner" width="1" height="1" patternUnits="userSpaceOnUse">
              <rect width="1" height="1" fill="#123456"/>
            </pattern>
          </defs>
          <rect width="8" height="8" fill="url(#outer)"/>
        </svg>"##;
        let tree = usvg::Tree::from_str(source, &usvg::Options::default()).unwrap();
        let (width, _, pixels) = render_simple(&tree, 1).expect("nested Source pattern");
        for (x, y) in [(0, 0), (5, 6)] {
            let offset = (y * width as usize + x) * 4;
            assert_eq!(&pixels[offset..offset + 4], &[0x12, 0x34, 0x56, 255]);
        }
    }

    #[test]
    fn translucent_pattern_paint_is_premultiplied_before_effect_group_compositing() {
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
          <defs><pattern id="p" x="0" y="0" width="2" height="2" patternUnits="userSpaceOnUse">
            <g opacity="0.5"><rect width="2" height="2" fill="#123456"/></g>
          </pattern></defs>
          <g opacity="0.8"><rect width="8" height="8" fill="url(#p)"/></g>
        </svg>"##;
        let tree = usvg::Tree::from_str(source, &usvg::Options::default()).unwrap();
        let (width, _, pixels) = render_simple(&tree, 1).expect("translucent pattern layers");
        let center = (4 * width as usize + 4) * 4;
        assert_eq!(&pixels[center..center + 4], &[160, 174, 187, 255]);
    }

    #[test]
    fn fill_implicitly_closes_open_lifeline_paths() {
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
          <rect width="8" height="8" fill="white"/>
          <path d="M 4 1 L 4 7" stroke="#0D32B2" stroke-width="2"/>
        </svg>"##;
        let tree = usvg::Tree::from_str(source, &usvg::Options::default()).unwrap();
        let (width, _, pixels) = render_simple(&tree, 1).expect("simple lifeline path");
        let pixel = |x: usize, y: usize| {
            let index = (y * width as usize + x) * 4;
            &pixels[index..index + 4]
        };

        assert_eq!(pixel(4, 4), &[13, 50, 178, 255]);
        assert_eq!(pixel(1, 4), &[255, 255, 255, 255]);
        assert_eq!(pixel(7, 4), &[255, 255, 255, 255]);
    }

    #[test]
    fn exact_dash_metadata_accepts_unique_lifeline_suffixes() {
        let dash = 12.0_f64;
        let gap = f64::from_bits(0x4027_ad72_e258_72d6);
        let offset = 0.0_f64;
        let id = format!(
            "d2rs-exact-dash-{:016x}-{:016x}-{:016x}-0000002a",
            dash.to_bits(),
            gap.to_bits(),
            offset.to_bits(),
        );
        assert_eq!(
            exact_dash_parameters_from_id(&id),
            Some((dash, gap, offset))
        );
        assert_eq!(
            exact_dash_parameters_from_id(&format!("{id}-salt-a")),
            Some((dash, gap, offset))
        );

        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20">
              <line id="{id}" x1="5" y1="1" x2="5" y2="19" stroke="blue"
                stroke-width="2" stroke-dasharray="12,11.838767"/>
            </svg>"#
        );
        let tree = usvg::Tree::from_str(&source, &usvg::Options::default()).unwrap();
        let path = tree
            .root()
            .children()
            .iter()
            .find_map(|node| match node {
                usvg::Node::Path(path) => Some(path),
                usvg::Node::Group(_) | usvg::Node::Image(_) | usvg::Node::Text(_) => None,
            })
            .expect("SVG line becomes a raster path");
        assert_eq!(exact_dash_parameters(path), Some((dash, gap, offset)));
    }

    #[test]
    fn animated_dash_split_uses_go_fused_interpolation_bits() {
        let gap = f64::from_bits(0x4045_28be_3ecd_14a3);
        let points = [Point64 { x: 84.5, y: 33.0 }, Point64 { x: 172.5, y: 33.0 }];
        let runs = Rasterizer::dashed_centerline_exact(
            &points,
            false,
            &[45.0, gap as f32],
            0.0,
            Some((45.0, gap, -1.375_579_720_545_541_3)),
        );
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].points.len(), 2);
        assert_eq!(runs[0].points[0].x.to_bits(), 0x4055_7809_7f86_322b);
        assert_eq!(runs[0].points[1].x.to_bits(), 0x4060_5c04_bfc3_1916);
    }

    #[test]
    fn rotated_containing_viewport_does_not_attenuate_polygon_border() {
        let svg = |clipped: bool| {
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="80">
          <defs><clipPath id="viewport"><rect width="10" height="12"/></clipPath></defs>
          <g transform="translate(40 30) rotate(123)" {}>
            <polygon points="0,0 10,6 0,12" fill="blue"/>
          </g>
        </svg>"##,
                if clipped {
                    "clip-path=\"url(#viewport)\""
                } else {
                    ""
                }
            )
        };
        let render = |clipped| {
            let tree = usvg::Tree::from_str(&svg(clipped), &usvg::Options::default()).unwrap();
            render_simple(&tree, 2).expect("native rotated viewport").2
        };
        assert_eq!(render(true), render(false));
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn opaque_arrow_mask_preserves_the_source_rgba_layer_boundary() {
        // These complete RGBA controls come from the pinned Go renderer,
        // with the same round stroke and triangle, at scale two. A white
        // mask changes antialiased pixels even though it removes no ink.
        for (masked, expected) in [
            (
                false,
                d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/opaque-arrow-direct.rgba"),
            ),
            (
                true,
                d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/opaque-arrow-masked.rgba"),
            ),
        ] {
            let mask = if masked { "mask=\"url(#mask)\"" } else { "" };
            let source = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="80">
                <defs><mask id="mask" maskUnits="userSpaceOnUse" x="0" y="0" width="80" height="80">
                  <rect width="80" height="80" fill="white"/>
                </mask></defs>
                <g {mask}>
                  <path d="M20 10 L20 54" fill="none" stroke="#0D32B2" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                  <path d="M14 47 L20 57 L26 47 Z" fill="#0D32B2"/>
                </g></svg>"##
            );
            let tree = usvg::Tree::from_str(&source, &usvg::Options::default()).unwrap();
            let (width, height, pixels) = render_simple(&tree, 2).expect("native solid arrow");
            assert_eq!((width, height), (160, 160));
            assert_eq!(pixels.len(), expected.len());
            let differences = pixels
                .chunks_exact(4)
                .zip(expected.chunks_exact(4))
                .enumerate()
                .filter(|(_, (actual, source))| actual != source)
                .map(|(index, (actual, source))| (index % 160, index / 160, actual, source))
                .collect::<Vec<_>>();
            assert!(differences.is_empty(), "masked={masked}: {differences:?}");
        }
    }

    #[test]
    fn masked_group_preserves_a_child_viewport_clip() {
        // A connection label creates a mask; a rotated arrowhead can also
        // have a nontrivial viewport clip. Processing the parent mask must
        // neither reject that clip nor suppress the child's own effects.
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="80">
          <defs>
            <mask id="mask" maskUnits="userSpaceOnUse" x="0" y="0" width="80" height="80">
              <rect width="80" height="80" fill="white"/>
              <rect x="35" y="0" width="10" height="80" fill="black"/>
            </mask>
            <clipPath id="clip"><rect x="20" y="20" width="40" height="40"/></clipPath>
          </defs>
          <g mask="url(#mask)"><g clip-path="url(#clip)">
            <rect width="80" height="80" fill="red"/>
          </g></g>
        </svg>"##;
        let tree = usvg::Tree::from_str(source, &usvg::Options::default()).unwrap();
        let (width, _, pixels) = render_simple(&tree, 1).expect("native mask plus clip");
        let pixel = |x: usize, y: usize| {
            let index = (y * width as usize + x) * 4;
            &pixels[index..index + 4]
        };
        assert_eq!(pixel(25, 25), &[255, 0, 0, 255]);
        assert_eq!(pixel(40, 40), &[255, 255, 255, 255]);
        assert_eq!(pixel(10, 40), &[255, 255, 255, 255]);
        assert_eq!(pixel(40, 70), &[255, 255, 255, 255]);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod color_placement_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn color_glyph_trees_use_source_grapheme_positions_and_primary_spaces() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/font-color-shaping-source-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let raw = row["text"].as_str().unwrap();
            let size = row["size"].as_f64().unwrap();
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="140" height="80"><text x="70.25" y="42.75" font-family="Source Sans Pro" font-size="{size}" text-anchor="middle">{raw}</text></svg>"#
            );
            let mut options = usvg::Options::default();
            crate::load_bundled_fonts_for_primary(&mut options, false, false);
            let resolver = std::sync::Mutex::new(crate::bundled_emoji::Resolver::default());
            options.font_resolver.select_fallback = Box::new(move |value, excluded, database| {
                resolver.lock().unwrap().select(value, excluded, database)
            });
            let tree = Tree::from_str(&svg, &options).unwrap();
            let usvg::Node::Text(text) = &tree.root().children()[0] else {
                panic!("text");
            };
            let expected = &row["shaped"];
            let source = expected["Glyphs"].as_array().unwrap();
            let layout = native_mixed_font_layout(&tree, text).unwrap();
            let (spans, transforms) = layout.as_ref();
            assert_eq!(spans[0].positioned_glyphs.len(), source.len(), "{row}");
            for ((glyph, transform), source) in spans[0]
                .positioned_glyphs
                .iter()
                .zip(&transforms[0])
                .zip(source)
            {
                assert_eq!(glyph.id.0, source["ID"].as_u64().unwrap() as u32, "{row}");
                let family = &tree.fontdb().face(glyph.font).unwrap().families[0].0;
                assert_eq!(
                    family,
                    if source["Face"] == 0 {
                        "Source Sans Pro"
                    } else {
                        "Noto Color Emoji"
                    },
                    "{row}"
                );
                assert_eq!(
                    transform.tx,
                    70.25 - expected["Advance"].as_f64().unwrap() / 2.0
                        + source["PositionX"].as_f64().unwrap(),
                    "{row}"
                );
                assert_eq!(
                    transform.ty,
                    42.75 + source["PositionY"].as_f64().unwrap(),
                    "{row}"
                );
            }
            let placements = native_color_glyph_placements(&tree, text);
            if raw.chars().any(|value| value.is_ascii_alphabetic()) {
                assert!(
                    placements.is_none(),
                    "mixed monochrome paint retains its existing path"
                );
            } else {
                let placements = placements.expect("all COLR groups are mapped");
                let colored = spans[0]
                    .positioned_glyphs
                    .iter()
                    .zip(&transforms[0])
                    .filter(|(glyph, _)| {
                        tree.fontdb().face(glyph.font).unwrap().families[0].0 == "Noto Color Emoji"
                    });
                assert_eq!(placements.len(), colored.clone().count(), "{row}");
                for ((node, correction), (_, expected)) in placements.into_iter().zip(colored) {
                    let usvg::Node::Group(group) = node else {
                        panic!("COLR group");
                    };
                    let actual = affine_mul(
                        glyph_transform64(correction),
                        glyph_transform64(group.transform()),
                    );
                    assert!(
                        (actual.tx - expected.tx).abs() < 0.00001
                            && (actual.ty - expected.ty).abs() < 0.00001,
                        "{row}: {},{} vs {},{}",
                        actual.tx,
                        actual.ty,
                        expected.tx,
                        expected.ty
                    );
                }
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod mixed_color_paint_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    use base64::Engine as _;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn mixed_color_full_frames_match_source_backgrounds_and_opacity() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/mixed-color-paint-source-v090.json"
        ))
        .unwrap();
        let mut failures = Vec::new();
        for (ordinal, row) in rows.as_array().unwrap().iter().enumerate() {
            assert_eq!(row["error"], "");
            let raw = row["text"].as_str().unwrap();
            let size = row["size"].as_f64().unwrap();
            let opacity = row["opacity"].as_f64().unwrap();
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="140" height="80"><text x="70.25" y="42.75" font-family="Source Sans Pro" font-size="{size}" text-anchor="middle" fill="#4b4e60">{raw}</text></svg>"##
            );
            let mut options = usvg::Options::default();
            crate::load_bundled_fonts_for_primary(&mut options, false, false);
            let resolver = std::sync::Mutex::new(crate::bundled_emoji::Resolver::default());
            options.font_resolver.select_fallback = Box::new(move |value, excluded, database| {
                resolver.lock().unwrap().select(value, excluded, database)
            });
            let tree = Tree::from_str(&svg, &options).unwrap();
            let usvg::Node::Text(text) = &tree.root().children()[0] else {
                panic!("text")
            };
            assert!(
                text_contains_color_glyph(&tree, text),
                "reconstructed color glyph must reach the native painter: {ordinal}"
            );
            let layout = native_mixed_font_layout(&tree, text).unwrap();
            let (spans, positions) = layout.as_ref();
            let expected_glyphs = row["shaped"]["Glyphs"].as_array().unwrap();
            assert_eq!(
                spans[0].positioned_glyphs.len(),
                expected_glyphs.len(),
                "row {ordinal}"
            );
            for ((glyph, position), source) in spans[0]
                .positioned_glyphs
                .iter()
                .zip(&positions[0])
                .zip(expected_glyphs)
            {
                assert_eq!(
                    glyph.id.0,
                    source["ID"].as_u64().unwrap() as u32,
                    "row {ordinal}"
                );
                assert_eq!(
                    tree.fontdb().face(glyph.font).unwrap().families[0].0,
                    if source["Face"] == 0 {
                        "Source Sans Pro"
                    } else {
                        "Noto Color Emoji"
                    },
                    "row {ordinal}"
                );

                assert_eq!(
                    position.tx,
                    70.25 - row["shaped"]["Advance"].as_f64().unwrap() / 2.0
                        + source["PositionX"].as_f64().unwrap(),
                    "row {ordinal}"
                );
                assert_eq!(
                    position.ty,
                    42.75 + source["PositionY"].as_f64().unwrap(),
                    "row {ordinal}"
                );
            }
            let background = base64::engine::general_purpose::STANDARD
                .decode(row["background"].as_str().unwrap())
                .unwrap();
            let premul: [u8; 4] = std::array::from_fn(|i| {
                if i == 3 {
                    background[3]
                } else {
                    ((u32::from(background[i]) * 257 * u32::from(background[3]) / 255) >> 8) as u8
                }
            });
            let mut actual = vec![0; 140 * 80 * 4];
            for p in actual.chunks_exact_mut(4) {
                p.copy_from_slice(&premul)
            }
            assert_eq!(
                draw_color_text_native(
                    &tree,
                    text,
                    &mut actual,
                    140,
                    80,
                    1.0,
                    opacity as f32,
                    None,
                ),
                Some(true),
                "{row}"
            );
            crate::unpremultiply_rgba(&mut actual);
            let expected = base64::engine::general_purpose::STANDARD
                .decode(row["pixels"].as_str().unwrap())
                .unwrap();
            assert_eq!(actual.len(), expected.len());
            let different = actual
                .chunks_exact(4)
                .zip(expected.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            if different != 0 {
                let first = actual
                    .chunks_exact(4)
                    .zip(expected.chunks_exact(4))
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .take(3)
                    .map(|(i, (a, b))| (i % 140, i / 140, a.to_vec(), b.to_vec()))
                    .collect::<Vec<_>>();
                failures.push((ordinal, raw.to_owned(), size, opacity, different, first));
            }
        }
        assert!(
            failures.is_empty(),
            "complete mixed Source frame differences: {failures:?}"
        );
    }
}

#[cfg(test)]
mod original_gradient_sample_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_gradient_transform_cone_spread_and_rgba_samples() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/gradient-samples-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 288);
        fn path(group: &usvg::Group) -> Option<&usvg::Path> {
            group.children().iter().find_map(|node| match node {
                usvg::Node::Path(path) => Some(path.as_ref()),
                usvg::Node::Group(group) => path(group),
                _ => None,
            })
        }
        for row in rows.as_array().unwrap() {
            let tree =
                usvg::Tree::from_str(row["svg"].as_str().unwrap(), &usvg::Options::default())
                    .unwrap();
            let path = path(tree.root()).unwrap();
            let fill = path.fill().unwrap();
            let gradient_transform = match fill.paint() {
                Paint::LinearGradient(g) => g.transform(),
                Paint::RadialGradient(g) => g.transform(),
                _ => panic!("expected gradient"),
            };
            let inv = gradient_inverse(gradient_transform, path.abs_transform()).unwrap();
            let native_inv = [inv.sx, inv.ky, inv.kx, inv.sy, inv.tx, inv.ty];
            for (i, n) in native_inv.iter().enumerate() {
                assert_eq!(
                    n.to_bits(),
                    u64::from_str_radix(row["inverse_bits"][i].as_str().unwrap(), 16).unwrap(),
                    "{} inverse {}",
                    row["name"],
                    i
                );
            }
            let raw = crate::svg_precision::gradient_stops64(row["svg"].as_str().unwrap());
            let tiles = HashMap::new();
            let prepared = paint_sampler(
                fill.paint(),
                fill.opacity().get(),
                path.abs_transform(),
                &tiles,
                None,
                None,
                Point64::default(),
                &raw,
            );
            let wanted = row["samples"].as_array().unwrap();
            assert_eq!(wanted.len(), 48 * 32);
            for (index, wanted) in wanted.iter().enumerate() {
                let actual = paint_color_at(
                    fill.paint(),
                    fill.opacity().get(),
                    path.abs_transform(),
                    index % 48,
                    index / 48,
                    &HashMap::new(),
                    None,
                    None,
                    Point64::default(),
                    &raw,
                );
                let expected = (!wanted.is_null())
                    .then(|| std::array::from_fn(|i| wanted[i].as_u64().unwrap() as u8));
                assert_eq!(actual, expected, "{} sample {}", row["name"], index);
                assert_eq!(
                    prepared(index % 48, index / 48),
                    expected,
                    "{} prepared sample {}",
                    row["name"],
                    index
                );
            }
        }
    }
}

#[cfg(test)]
mod original_ascii_positioning_contract {
    #[cfg(feature = "oracle-fixtures")]
    use resvg::usvg;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn ascii_scene_glyph_origins_match_original_go() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/ascii-shaping-source-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let style = match row["font"].as_str().unwrap() {
                "Regular" => "",
                "Bold" => "font-weight=\"bold\"",
                "Italic" => "font-style=\"italic\"",
                _ => unreachable!(),
            };
            let body = row["text"]
                .as_str()
                .unwrap()
                .replace('&', "&amp;")
                .replace('<', "&lt;");
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="280" height="80"><text x="140" y="42" text-anchor="middle" font-family="Source Sans Pro" font-size="{}" {}>{}</text></svg>"#,
                row["size"], style, body
            );
            let mut options = usvg::Options::default();
            super::super::load_bundled_fonts(&mut options);
            let tree = usvg::Tree::from_str(&svg, &options).unwrap();
            let text = tree
                .root()
                .children()
                .iter()
                .find_map(|n| match n {
                    usvg::Node::Text(t) => Some(t.as_ref()),
                    _ => None,
                })
                .unwrap();
            let actual = super::fixed_glyph_transforms(&tree, text, text.layouted()).unwrap();
            let expected = &row["shaped"];
            let actual = actual.iter().flatten().collect::<Vec<_>>();
            assert_eq!(actual.len(), expected["Glyphs"].as_array().unwrap().len());
            for (i, (g, e)) in actual
                .iter()
                .zip(expected["Glyphs"].as_array().unwrap())
                .enumerate()
            {
                assert_eq!(
                    (g.tx, g.ty),
                    (
                        140.0 - expected["Advance"].as_f64().unwrap() / 2.0
                            + e["PositionX"].as_f64().unwrap(),
                        42.0 + e["PositionY"].as_f64().unwrap()
                    ),
                    "{} {} {} glyph {}",
                    row["font"],
                    row["size"],
                    row["text"],
                    i
                );
            }
        }
    }
}

#[cfg(test)]
mod native_prepared_bounds_contract {
    use super::*;

    #[test]
    fn viewport_bounds_union_children_before_clipping_and_reference_transport() {
        // Original d2raster/raster.go prepares the child union before the
        // viewport clip; bands.go then intersects that prepared group with
        // its parent's paint bounds. A gap between children must survive.
        for viewbox in ["0 0 100 100", "0.000 0.000 100.000 100.000", "0,0,100,100"] {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
                <g id="opacity-parent" opacity="0.37">
                <svg id="d2-rs-native-md-viewport-test" data-d2-rs-markdown-reference="ordinary" width="100" height="100" viewBox="{viewbox}">
                <rect id="d2-rs-native-md-rect-first" x="10" y="10" width="10" height="10"/>
                <rect id="d2-rs-native-md-rect-last" x="10" y="150" width="10" height="10"/>
                </svg></g></svg>"#
            );
            let (svg, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let tree = Tree::from_str(&svg, &usvg::Options::default()).unwrap();
            fn find<'a>(group: &'a usvg::Group, id: &str) -> Option<&'a usvg::Group> {
                if group.id() == id {
                    return Some(group);
                }
                group.children().iter().find_map(|node| match node {
                    usvg::Node::Group(child) => find(child, id),
                    _ => None,
                })
            }
            let viewport = find(tree.root(), "d2-rs-native-md-viewport-test").unwrap();
            let opacity = find(tree.root(), "opacity-parent").unwrap();
            let canvas = TextPaintBounds {
                left: 0,
                top: 0,
                right: 200,
                bottom: 200,
            };
            let expected = TextPaintBounds {
                left: 9,
                top: 9,
                right: 21,
                bottom: 101,
            };
            let bounds = native_paint_bounds(
                &tree,
                viewport,
                Some([0.0, 0.0, 100.0, 100.0]),
                1.0,
                canvas,
                &sources,
            )
            .unwrap();
            assert_eq!(bounds, expected, "{viewbox}");
            assert_eq!(
                native_paint_bounds(&tree, opacity, None, 1.0, canvas, &sources),
                Some(expected)
            );
            assert_eq!(
                native_paint_bounds(
                    &tree,
                    viewport,
                    Some([0.0, 0.0, 100.0, 100.0]),
                    1.0,
                    bounds,
                    &sources
                ),
                Some(expected)
            );
            let gap = TextPaintBounds {
                left: 0,
                top: 30,
                right: 200,
                bottom: 80,
            };
            assert_eq!(
                native_paint_bounds(&tree, opacity, None, 1.0, gap, &sources),
                Some(TextPaintBounds {
                    left: 9,
                    top: 30,
                    right: 21,
                    bottom: 80
                })
            );
        }
    }
}

#[cfg(test)]
mod original_raster_band_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    use sha2::{Digest, Sha256};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_go_effect_frames_match_across_band_boundaries() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/raster-bands-source-v090.json"
        ))
        .unwrap();
        let rows = fixture["cases"].as_array().unwrap();
        assert_eq!(rows.len(), 42);
        let mut options = usvg::Options::default();
        crate::load_bundled_fonts(&mut options);
        let mut failures = Vec::new();
        for row in rows {
            let (svg, sources) =
                crate::svg_precision::tag_embedded_svg_images(row["svg"].as_str().unwrap());
            let tree = usvg::Tree::from_str(&svg, &options).unwrap();
            let band = row["band_height"].as_u64().unwrap() as usize;
            let result = render_simple_with_band_height(
                &tree,
                row["scale"].as_u64().unwrap() as u32,
                &[],
                &sources,
                [255; 4],
                &BTreeMap::new(),
                (band != 0).then_some(band),
                None,
            );
            match result {
                None => failures.push(format!(
                    "{} scale{} band{band}: unsupported",
                    row["mode"], row["scale"]
                )),
                Some((w, h, bytes)) => {
                    assert_eq!(
                        (w as u64, h as u64),
                        (
                            row["width"].as_u64().unwrap(),
                            row["height"].as_u64().unwrap()
                        )
                    );
                    let hash = format!("{:x}", Sha256::digest(&bytes));
                    if hash != row["rgba_sha256"].as_str().unwrap() {
                        failures.push(format!(
                            "{} scale{} band{band}: {hash}",
                            row["mode"], row["scale"]
                        ));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "complete Source frame failures: {failures:?}"
        );
    }
}

#[cfg(test)]
mod original_imported_primitive_contract {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_imported_ellipse_circle_polygon_and_polyline_frames() {
        use base64::Engine as _;
        use sha2::{Digest, Sha256};
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/imported-primitives-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 12);
        for row in rows {
            let encoded = base64::engine::general_purpose::STANDARD
                .encode(row["asset_svg"].as_str().unwrap());
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="192" height="192"><image x="16" y="32" width="128" height="128" href="data:image/svg+xml;base64,{encoded}"/></svg>"#
            );
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let tree =
                resvg::usvg::Tree::from_str(&tagged, &resvg::usvg::Options::default()).unwrap();
            let (width, height, pixels) = super::render_simple_with_sources(
                &tree,
                row["scale"].as_u64().unwrap() as u32,
                &[],
                &sources,
            )
            .expect("native imported primitive rendering");
            assert_eq!(width, row["width"].as_u64().unwrap() as u32, "{row}");
            assert_eq!(height, row["height"].as_u64().unwrap() as u32, "{row}");
            assert_eq!(
                format!("{:x}", Sha256::digest(&pixels)),
                row["rgba_sha256"].as_str().unwrap(),
                "{row}"
            );
        }
    }
}

#[cfg(test)]
mod original_math_vector_contract {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_intrinsic_math_vector_frames() {
        use base64::Engine as _;
        use sha2::{Digest, Sha256};
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/math-vector-frames-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 12);
        let mut failures = Vec::new();
        for row in rows {
            for viewport_zoom in [false, true] {
                let original = row["asset_svg"].as_str().unwrap();
                let document = roxmltree::Document::parse(original).unwrap();
                let root = document.root_element();
                let width = row["intrinsic_width"].as_f64().unwrap();
                let height = row["intrinsic_height"].as_f64().unwrap();
                let asset = original
                    .replacen(
                        &format!(" width=\"{}\"", root.attribute("width").unwrap()),
                        &format!(" width=\"{width}\""),
                        1,
                    )
                    .replacen(
                        &format!(" height=\"{}\"", root.attribute("height").unwrap()),
                        &format!(" height=\"{height}\""),
                        1,
                    )
                    .replacen(
                        "<svg ",
                        "<svg data-d2-rs-intrinsic-math=\"ordinary\" color=\"#0A0F25\" ",
                        1,
                    );
                let encoded = base64::engine::general_purpose::STANDARD.encode(asset);
                let svg = format!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" width="192" height="160" viewBox="0 0 192 160" preserveAspectRatio="xMinYMin meet"><svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="192" height="160" viewBox="0 0 192 160"><rect width="192" height="160" fill="white"/><image id="math" x="20.25" y="40.5" width="{width}" height="{height}" href="data:image/svg+xml;base64,{encoded}"/></svg></svg>"#
                );
                let source_scale = row["scale"].as_u64().unwrap() as u32;
                let (svg, raster_scale) = if viewport_zoom {
                    (
                        svg.replacen(
                            "width=\"192\" height=\"160\"",
                            &format!(
                                "width=\"{}\" height=\"{}\"",
                                192 * source_scale,
                                160 * source_scale
                            ),
                            1,
                        ),
                        1,
                    )
                } else {
                    (svg, source_scale)
                };
                let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
                let tree =
                    resvg::usvg::Tree::from_str(&tagged, &resvg::usvg::Options::default()).unwrap();
                let (w, h, pixels) =
                    super::render_simple_with_sources(&tree, raster_scale, &[], &sources)
                        .expect("native math vector import");
                assert_eq!(
                    (w, h),
                    (
                        row["width"].as_u64().unwrap() as u32,
                        row["height"].as_u64().unwrap() as u32
                    )
                );
                if !viewport_zoom && let Some(directory) = std::env::var_os("D2_RS_TEST_PIXEL_DUMP")
                {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    let file = std::fs::File::create(
                        directory.join(format!("{}.png", row["name"].as_str().unwrap())),
                    )
                    .unwrap();
                    let mut encoder = png::Encoder::new(file, w, h);
                    encoder.set_color(png::ColorType::Rgba);
                    encoder.set_depth(png::BitDepth::Eight);
                    encoder
                        .write_header()
                        .unwrap()
                        .write_image_data(&pixels)
                        .unwrap();
                }
                let actual = format!("{:x}", Sha256::digest(pixels));
                if actual != row["rgba_sha256"].as_str().unwrap() {
                    failures.push(format!("{}: {actual}", row["name"]));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "complete math frame failures: {failures:?}"
        );
    }
}

#[cfg(test)]
mod original_nested_math_clip_contract {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_nested_math_clip_frames() {
        use base64::Engine as _;
        use sha2::{Digest, Sha256};
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/nested-math-clip-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 24);
        let mut failures = Vec::new();
        for row in rows {
            for viewport_zoom in [false, true] {
                let original = row["asset_svg"].as_str().unwrap();
                let document = roxmltree::Document::parse(original).unwrap();
                let root = document.root_element();
                let width = row["intrinsic_width"].as_f64().unwrap();
                let height = row["intrinsic_height"].as_f64().unwrap();
                let asset = original
                    .replacen(
                        &format!(" width=\"{}\"", root.attribute("width").unwrap()),
                        &format!(" width=\"{width}\""),
                        1,
                    )
                    .replacen(
                        &format!(" height=\"{}\"", root.attribute("height").unwrap()),
                        &format!(" height=\"{height}\""),
                        1,
                    )
                    .replacen(
                        "<svg ",
                        "<svg data-d2-rs-intrinsic-math=\"ordinary\" color=\"#0A0F25\" ",
                        1,
                    );
                let encoded = base64::engine::general_purpose::STANDARD.encode(asset);
                let svg = format!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" width="192" height="160" viewBox="0 0 192 160" preserveAspectRatio="xMinYMin meet"><svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="192" height="160" viewBox="0 0 192 160"><rect width="192" height="160" fill="white"/><image id="math" x="20.25" y="40.5" width="{width}" height="{height}" href="data:image/svg+xml;base64,{encoded}"/></svg></svg>"#
                );
                let source_scale = row["scale"].as_u64().unwrap() as u32;
                let (svg, raster_scale) = if viewport_zoom {
                    (
                        svg.replacen(
                            "width=\"192\" height=\"160\"",
                            &format!(
                                "width=\"{}\" height=\"{}\"",
                                192 * source_scale,
                                160 * source_scale
                            ),
                            1,
                        ),
                        1,
                    )
                } else {
                    (svg, source_scale)
                };
                let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
                let tree =
                    resvg::usvg::Tree::from_str(&tagged, &resvg::usvg::Options::default()).unwrap();
                let (w, h, pixels) =
                    super::render_simple_with_sources(&tree, raster_scale, &[], &sources)
                        .expect("native math vector import");
                assert_eq!(
                    (w, h),
                    (
                        row["width"].as_u64().unwrap() as u32,
                        row["height"].as_u64().unwrap() as u32
                    )
                );
                if !viewport_zoom && let Some(directory) = std::env::var_os("D2_RS_TEST_PIXEL_DUMP")
                {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    let file = std::fs::File::create(
                        directory.join(format!("{}.png", row["name"].as_str().unwrap())),
                    )
                    .unwrap();
                    let mut encoder = png::Encoder::new(file, w, h);
                    encoder.set_color(png::ColorType::Rgba);
                    encoder.set_depth(png::BitDepth::Eight);
                    encoder
                        .write_header()
                        .unwrap()
                        .write_image_data(&pixels)
                        .unwrap();
                }
                let actual = format!("{:x}", Sha256::digest(pixels));
                if actual != row["rgba_sha256"].as_str().unwrap() {
                    failures.push(format!("{}: {actual}", row["name"]));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "complete math frame failures: {failures:?}"
        );
    }
}

#[cfg(test)]
mod original_empty_svg_ink_contract {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_empty_svg_ink_frames() {
        use base64::Engine as _;
        use sha2::{Digest, Sha256};
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/empty-svg-ink-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 24);
        let mut failures = Vec::new();
        for row in rows {
            for viewport_zoom in [false, true] {
                let original = row["asset_svg"].as_str().unwrap();
                let document = roxmltree::Document::parse(original).unwrap();
                let root = document.root_element();
                let width = row["intrinsic_width"].as_f64().unwrap();
                let height = row["intrinsic_height"].as_f64().unwrap();
                let asset = original
                    .replacen(
                        &format!(" width=\"{}\"", root.attribute("width").unwrap()),
                        &format!(" width=\"{width}\""),
                        1,
                    )
                    .replacen(
                        &format!(" height=\"{}\"", root.attribute("height").unwrap()),
                        &format!(" height=\"{height}\""),
                        1,
                    )
                    .replacen(
                        "<svg ",
                        "<svg data-d2-rs-intrinsic-math=\"ordinary\" color=\"#0A0F25\" ",
                        1,
                    );
                let encoded = base64::engine::general_purpose::STANDARD.encode(asset);
                let svg = format!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" width="192" height="160" viewBox="0 0 192 160" preserveAspectRatio="xMinYMin meet"><svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="192" height="160" viewBox="0 0 192 160"><rect width="192" height="160" fill="white"/><image id="math" x="20.25" y="40.5" width="{width}" height="{height}" href="data:image/svg+xml;base64,{encoded}"/></svg></svg>"#
                );
                let source_scale = row["scale"].as_u64().unwrap() as u32;
                let (svg, raster_scale) = if viewport_zoom {
                    (
                        svg.replacen(
                            "width=\"192\" height=\"160\"",
                            &format!(
                                "width=\"{}\" height=\"{}\"",
                                192 * source_scale,
                                160 * source_scale
                            ),
                            1,
                        ),
                        1,
                    )
                } else {
                    (svg, source_scale)
                };
                let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
                let tree =
                    resvg::usvg::Tree::from_str(&tagged, &resvg::usvg::Options::default()).unwrap();
                let (w, h, pixels) =
                    super::render_simple_with_sources(&tree, raster_scale, &[], &sources)
                        .expect("native math vector import");
                assert_eq!(
                    (w, h),
                    (
                        row["width"].as_u64().unwrap() as u32,
                        row["height"].as_u64().unwrap() as u32
                    )
                );
                if !viewport_zoom && let Some(directory) = std::env::var_os("D2_RS_TEST_PIXEL_DUMP")
                {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    let file = std::fs::File::create(
                        directory.join(format!("{}.png", row["name"].as_str().unwrap())),
                    )
                    .unwrap();
                    let mut encoder = png::Encoder::new(file, w, h);
                    assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
                    let rgb: Vec<u8> = pixels
                        .chunks_exact(4)
                        .flat_map(|pixel| pixel[..3].iter().copied())
                        .collect();
                    encoder.set_color(png::ColorType::Rgb);
                    encoder.set_depth(png::BitDepth::Eight);
                    encoder
                        .write_header()
                        .unwrap()
                        .write_image_data(&rgb)
                        .unwrap();
                }
                let actual = format!("{:x}", Sha256::digest(pixels));
                if actual != row["rgba_sha256"].as_str().unwrap() {
                    failures.push(format!("{}: {actual}", row["name"]));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "complete math frame failures: {failures:?}"
        );
    }
}

#[cfg(test)]
mod markdown_path_crop_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    use sha2::{Digest, Sha256};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_markdown_clip_pixel_inventory() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-clip-pixels-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 96);
        let mut options = usvg::Options::default();
        crate::load_bundled_fonts(&mut options);
        let mut results = Vec::new();
        let mut failures = Vec::new();
        for (index, row) in rows.as_array().unwrap().iter().enumerate() {
            let (svg, sources) =
                crate::svg_precision::tag_embedded_svg_images(row["svg"].as_str().unwrap());
            let tree = usvg::Tree::from_str(&svg, &options).unwrap();
            let (width, height, pixels) = render_simple_with_sources(
                &tree,
                row["scale"].as_u64().unwrap() as u32,
                &[],
                &sources,
            )
            .expect("complete supported Source viewport frame");
            let hash = format!("{:x}", Sha256::digest(&pixels));
            let matched = u64::from(width) == row["width"].as_u64().unwrap()
                && u64::from(height) == row["height"].as_u64().unwrap()
                && hash == row["rgba_sha256"].as_str().unwrap();
            if !matched {
                failures.push(index);
            }
            results.push(serde_json::json!({"index":index,"width":width,"height":height,"rgba_sha256":hash,"matched":matched}));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_MD_CLIP_PIXELS_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&results).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "complete Source clip frame differences: {failures:?}"
        );
    }
}

#[path = "owned_png_scene.rs"]
mod owned_png_scene;
pub(crate) use owned_png_scene::owned_webp_asset;

#[cfg(all(test, feature = "oracle-fixtures"))]
mod touched_interval_source_contract {
    use super::*;
    #[derive(Debug, PartialEq, serde::Deserialize)]
    struct State {
        min: usize,
        max: usize,
        touched: bool,
        partial: Vec<u32>,
        difference: Vec<u32>,
    }
    #[derive(serde::Deserialize)]
    struct Step {
        row: usize,
        before: State,
        after: State,
        x: u64,
        y: u64,
    }
    #[derive(serde::Deserialize)]
    struct Record {
        width: usize,
        x: f64,
        y: f64,
        slope: f64,
        winding: i8,
        steps: Vec<Step>,
    }
    fn state(r: &Rasterizer) -> State {
        State {
            min: r.touched_min,
            max: r.touched_max,
            touched: r.touched,
            partial: r.partial.iter().map(|v| v.to_bits()).collect(),
            difference: r.difference.iter().map(|v| v.to_bits()).collect(),
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_accumulation_intervals_bits_and_sentinel_cleanup() {
        // Original internal/scanline accumulateEdgeRow and clearTouched, both
        // rows retained for every edge. No tolerance or signed-zero folding.
        let records: Vec<Record> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/scanline-touched-source-v090.json"
        ))
        .unwrap();
        assert_eq!(records.len(), 1680);
        for (index, record) in records.into_iter().enumerate() {
            let mut raster = Rasterizer::new(record.width, 2, 0.375);
            raster.ensure_scan_buffers();
            raster.scan_edges.push(ScanEdge {
                end: 2,
                next: 0,
                x: record.x,
                y: record.y,
                bottom: 1.75,
                slope: record.slope,
                inverse_slope: if record.slope == 0. {
                    0.
                } else {
                    1. / record.slope
                },
                winding: f64::from(record.winding),
            });
            for step in record.steps {
                raster.accumulate_edge_row(0, step.row);
                assert_eq!(
                    state(&raster),
                    step.before,
                    "source edge {index} row {} before clear",
                    step.row
                );
                assert_eq!(
                    raster.scan_edges[0].x.to_bits(),
                    step.x,
                    "source edge {index} row {} x",
                    step.row
                );
                assert_eq!(
                    raster.scan_edges[0].y.to_bits(),
                    step.y,
                    "source edge {index} row {} y",
                    step.row
                );
                raster.clear_touched();
                assert_eq!(
                    state(&raster),
                    step.after,
                    "source edge {index} row {} after clear",
                    step.row
                );
            }
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod solid_span_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    use base64::Engine as _;
    #[derive(serde::Deserialize)]
    struct Record {
        width: usize,
        contours: Vec<Vec<[f64; 2]>>,
        color: [u8; 4],
        background: [u8; 4],
        pixels: String,
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_sparse_dense_and_partial_alpha_complete_frames() {
        let records: Vec<Record> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/scanline-touched-paint-source-v090.json"
        ))
        .unwrap();
        assert_eq!(records.len(), 120);
        for (index, record) in records.into_iter().enumerate() {
            let mut raster = Rasterizer::new(record.width, 8, 0.375);
            for points in record.contours {
                let points: Vec<_> = points.into_iter().map(|[x, y]| Point64 { x, y }).collect();
                raster.add_fill_polyline(&points, true);
            }
            let mut pixels = record.background.repeat(record.width * 8);
            raster.paint(&mut pixels, record.color);
            let expected = base64::engine::general_purpose::STANDARD
                .decode(record.pixels)
                .unwrap();
            assert!(
                pixels == expected,
                "original DrawRGBA frame {index}: first differing byte {:?}",
                pixels.iter().zip(&expected).position(|(a, b)| a != b)
            );
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod original_fallback_occurrence_order_contract {
    use super::*;
    #[test]
    fn raw_control_fallback_faces_follow_source_text_order() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><text x="87" y="79.25" font-family="Source Sans Pro" font-size="20" xml:space="preserve">var&#xD;&#x9;char</text></svg>"#;
        let mut options = usvg::Options::default();
        crate::load_bundled_fonts_for_primary(&mut options, false, false);
        options.fontdb_mut().load_system_fonts();
        let tree = Tree::from_str(svg, &options).unwrap();
        let usvg::Node::Text(text) = &tree.root().children()[0] else {
            panic!("native text")
        };
        let layout = native_mixed_font_layout_with_text(&tree, text, Some("var\r\tchar")).unwrap();
        let (spans, positions) = layout.as_ref();
        let glyphs = spans
            .iter()
            .flat_map(|span| span.positioned_glyphs.iter())
            .collect::<Vec<_>>();
        let positions = positions.iter().flatten().collect::<Vec<_>>();
        // Actual original Go scene glyph trace on this host, size20. The
        // fallback for CR supplies glyph2 before TAB adds its spacing face.
        let expected = [
            (49, 0.),
            (28, 8.953125),
            (45, 19.03125),
            (2, 25.96875),
            (1, 31.171875),
            (30, 37.828125),
            (35, 46.953125),
            (28, 57.828125),
            (45, 67.90625),
        ];
        assert_eq!(glyphs.len(), expected.len());
        assert_eq!(positions.len(), expected.len());
        for ((glyph, position), (id, x)) in glyphs.iter().zip(positions).zip(expected) {
            assert_eq!(glyph.id.0, id);
            assert_eq!(position.tx, 87. + x);
            assert_eq!(position.ty, 79.25);
        }
    }
}

#[cfg(test)]
mod clip_coverage_source_contract {
    use super::*;
    #[test]
    fn enclosing_bounds_do_not_discard_source_rounded_or_polygon_clips() {
        // effects.go renderEffectNode applies nonrectangular clips even when
        // every shape has the same enclosing box as the clipped image.
        for (data, covering) in [
            ("M0 0L100 0L100 100L0 100Z", true),
            ("M0 0L100 0L100 100L0 100L0 0Z", true),
            (
                "M20 0L80 0Q100 0 100 20L100 80Q100 100 80 100L20 100Q0 100 0 80L0 20Q0 0 20 0Z",
                false,
            ),
            (
                "M0 20L0 20C0 20 0 0 20 0L80 0L80 0C80 0 100 0 100 20L100 80C100 80 100 100 80 100L20 100C20 100 0 100 0 80Z",
                false,
            ),
            ("M0 0L100 0L50 100Z", false),
            ("M0 0L100 0L100 100L0 100ZM20 20L80 20L80 80L20 80Z", false),
        ] {
            let svg = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'><defs><clipPath id='clip'><path d='{data}'/></clipPath></defs><g clip-path='url(#clip)'><rect width='100' height='100'/></g></svg>"
            );
            let tree = Tree::from_str(&svg, &usvg::Options::default()).unwrap();
            let group = tree
                .root()
                .children()
                .iter()
                .find_map(|node| match node {
                    usvg::Node::Group(group) if group.clip_path().is_some() => Some(group),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                is_trivial_clip(group.clip_path().unwrap(), group),
                covering,
                "{data}"
            );
        }
    }
}

#[cfg(test)]
mod legend_shape_shadow_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    use sha2::{Digest, Sha256};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_complete_fractional_shadow_frames() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/legend-shadows-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 24);
        let mut mismatches = Vec::new();
        let capture = std::env::var_os("D2_RS_TEST_SHADOW_OUTPUT").map(std::path::PathBuf::from);
        if let Some(root) = &capture {
            std::fs::create_dir_all(root).unwrap();
        }
        for row in rows.as_array().unwrap() {
            let local_scale = row["local_scale"].as_f64().unwrap();
            let opacity = row["opacity"].as_f64().unwrap();
            let radius = row["radius"].as_f64().unwrap();
            let scale = row["scale"].as_u64().unwrap() as u32;
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="96" viewBox="0 0 128 96" preserveAspectRatio="xMinYMin meet"><defs><filter id="ls" x="-100%" y="-100%" width="300%" height="300%" color-interpolation-filters="sRGB"><feDropShadow dx="3" dy="5" stdDeviation="1.7" flood-color="#3d4574" flood-opacity="0.4"/></filter></defs><svg id="d2-rs-native-canvas-viewport-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="128" height="96" viewBox="0 0 128 96"><g id="d2-rs-native-legend-shape-icon-0" stroke-linecap="butt" stroke-linejoin="round" transform="translate(17.25 13.125) scale({local_scale})"><g opacity="{opacity}"><g id="d2-rs-native-legend-shape-shadow-0" data-d2-rs-legend-shape-shadow="scene" filter="url(#ls)"><rect x="0" y="0" width="120" height="80" rx="{radius}" ry="{radius}" fill="#f7f8fe" stroke="#0d32b2" stroke-width="2"/></g></g></g></svg></svg>"##
            );
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            assert!(sources.is_native_legend_shadow("d2-rs-native-legend-shape-shadow-0"));
            let tree =
                resvg::usvg::Tree::from_str(&tagged, &resvg::usvg::Options::default()).unwrap();
            let (width, height, pixels) =
                super::render_simple_with_sources(&tree, scale, &[], &sources)
                    .expect("source shadow uses native raster, no fallback");
            assert_eq!(
                (width, height),
                (
                    row["width"].as_u64().unwrap() as u32,
                    row["height"].as_u64().unwrap() as u32
                )
            );
            let digest = format!("{:x}", Sha256::digest(&pixels));
            let name = row["name"].as_str().unwrap();
            if let Some(root) = &capture {
                std::fs::write(root.join(format!("{name}.rgba")), &pixels).unwrap();
                std::fs::write(root.join(format!("{name}.svg")), &svg).unwrap();
            }
            if digest != row["rgba_sha256"].as_str().unwrap() {
                mismatches.push((name.to_owned(), digest));
            }
        }
        assert!(
            mismatches.is_empty(),
            "Complete source shadow frame mismatches: {mismatches:?}"
        );
    }
}

#[cfg(test)]
mod curve_tolerance_boundary_contract {
    use super::*;

    #[test]
    fn glyph_quadratic_preserves_go_control_distance_at_tolerance() {
        // Source Code Pro's 8 in stable/chaos2, theme 300. Captured from
        // master geometry.go:274–290 and controlsProgress:385–417, Go 1.27.
        // Separate products give exactly 0.125; Go ARM64's FMSUBD gives
        // 0.12500000000000003 and retains the extra subdivision.
        let mut points = Vec::new();
        let end = Point64 {
            x: 295.640625,
            y: 598.0625,
        };
        flatten_quadratic(
            Point64 {
                x: 294.828125,
                y: 597.453125,
            },
            Point64 {
                x: 295.140625,
                y: 597.84375,
            },
            end,
            0.125,
            0,
            &mut points,
        );
        #[cfg(target_arch = "aarch64")]
        let expected = vec![
            Point64 {
                x: 295.1875,
                y: 597.80078125,
            },
            end,
        ];
        #[cfg(not(target_arch = "aarch64"))]
        let expected = vec![end];
        assert_eq!(points.len(), expected.len());
        for (actual, expected) in points.iter().zip(&expected) {
            assert_eq!(
                (actual.x.to_bits(), actual.y.to_bits()),
                (expected.x.to_bits(), expected.y.to_bits())
            );
        }
    }
}

#[cfg(test)]
mod shadow_scan_tests {
    /// The word-wise scan bounds every pixel with a nonzero byte, exactly.
    #[test]
    fn nonzero_pixel_bounds_match_a_byte_scan() {
        for width in [1, 2, 3, 5, 8, 13] {
            let height = 7;
            for seed in 0..60_usize {
                let mut surface = vec![0_u8; width * height * 4];
                for k in 0..(seed % 4) {
                    let index = (seed * 131 + k * 977) % surface.len();
                    surface[index] = (seed + k) as u8 | 1;
                }
                let mut expected: Option<(usize, usize, usize, usize)> = None;
                for (index, &byte) in surface.iter().enumerate() {
                    if byte != 0 {
                        let (x, y) = (index / 4 % width, index / 4 / width);
                        expected = Some(match expected {
                            None => (x, y, x, y),
                            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                        });
                    }
                }
                assert_eq!(super::nonzero_pixel_bounds(&surface, width), expected);
            }
        }
    }
}
