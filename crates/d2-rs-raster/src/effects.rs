// SPDX-License-Identifier: MPL-2.0
//! d2raster/effects.go: offscreen accounting, resource planning, effect
//! layers (opacity, blend, clip, mask), even-odd supersampling and layer
//! compositing. Go's buffer cache only reuses allocations; live/peak
//! accounting (the observable part) is kept exactly. Contraction sites in
//! the blend compositors were read from their ARM64 assembly.

use d2_rs_scene::{BlendMode, FillRule, MaskType, Matrix, go_max, go_min};

use crate::fma::{go_fmadd, go_fmsub, go_fnmsub, translate};
use crate::geometry::{
    PreparedStroke, StrokeRun, Subpath, add_fill_subpath, add_stroke_run,
    painted_stroke_pixel_bounds, shifted, subpath_pixel_bounds,
};
use crate::paint::{PaintKind, linear_to_srgb, premultiplied_srgb_byte_to_linear, rounded_byte};
use crate::pixels::{AlphaBuf, Pt, Rect, Rgba, RgbaBuf};
use crate::raster::{
    PREPARED_COLRV1_SOFT_LIGHT, PreparedMask, PreparedNode, PreparedPrimitive, Scratch,
    draw_primitive, render_node,
};
use crate::regions::{effect_render_bounds, mask_render_bounds};
use crate::scanline::{self, Rasterizer, ScanError, WalkError, quantize_coverage};

pub const EVEN_ODD_SAMPLES_PER_AXIS: i64 = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderResources {
    pub peak_offscreen_bytes: i64,
    pub even_odd_clip_work: i64,
    pub scanline_work: i64,
    pub rasterizer_bytes: i64,
    pub rasterizer_width: i64,
    pub rasterizer_height: i64,
    pub rasterizer_edges: i64,
}

pub struct RasterizerResourcePlanner {
    pub counter: Rasterizer,
    pub region_filters: bool,
}

/// offscreenBudget: live and peak pixel-storage reservations.
#[derive(Clone, Copy, Debug, Default)]
pub struct OffscreenBudget {
    pub limit: i64,
    pub live: i64,
    pub peak: i64,
}

impl OffscreenBudget {
    pub fn reserve_bytes(&mut self, bytes: i64, purpose: &str) -> Result<i64, String> {
        if bytes > self.limit - self.live {
            return Err(format!(
                "d2raster: offscreen {purpose} requires {bytes} bytes with {} bytes already live, exceeding limit {}",
                self.live, self.limit
            ));
        }
        self.live += bytes;
        if self.live > self.peak {
            self.peak = self.live;
        }
        Ok(bytes)
    }

    pub fn release(&mut self, bytes: i64) {
        self.live -= bytes;
    }

    pub fn new_rgba(&mut self, bounds: Rect, purpose: &str) -> Result<(RgbaBuf, i64), String> {
        let bytes =
            pixel_storage_bytes(bounds, 4).map_err(|e| format!("d2raster: {purpose}: {e}"))?;
        let reservation = self.reserve_bytes(bytes, purpose)?;
        Ok((RgbaBuf::new(bounds), reservation))
    }

    pub fn new_alpha(&mut self, bounds: Rect, purpose: &str) -> Result<(AlphaBuf, i64), String> {
        let bytes =
            pixel_storage_bytes(bounds, 1).map_err(|e| format!("d2raster: {purpose}: {e}"))?;
        let reservation = self.reserve_bytes(bytes, purpose)?;
        Ok((AlphaBuf::new(bounds), reservation))
    }
}

pub fn pixel_storage_bytes(bounds: Rect, bytes_per_pixel: i64) -> Result<i64, String> {
    if bounds.empty() {
        return Ok(0);
    }
    let pixels = checked_multiply(bounds.dx(), bounds.dy())
        .ok_or("offscreen pixel storage exceeds the int64 domain")?;
    let bytes = checked_multiply(pixels, bytes_per_pixel)
        .ok_or("offscreen pixel storage exceeds the platform integer domain")?;
    Ok(bytes)
}

pub fn checked_multiply(left: i64, right: i64) -> Option<i64> {
    if left < 0 || right < 0 || left != 0 && right > i64::MAX / left {
        return None;
    }
    Some(left * right)
}

pub fn checked_add(left: i64, right: i64) -> Option<i64> {
    if left < 0 || right < 0 || right > i64::MAX - left {
        return None;
    }
    Some(left + right)
}

const PEAK_DOMAIN: &str = "d2raster: peak offscreen pixel storage exceeds the int64 domain";

fn add_resource_work(resources: &mut RenderResources, work: i64) -> Result<(), String> {
    resources.even_odd_clip_work = checked_add(resources.even_odd_clip_work, work)
        .ok_or("d2raster: even-odd clip work exceeds the int64 domain")?;
    Ok(())
}

fn add_scanline_work(resources: &mut RenderResources, work: i64) -> Result<(), String> {
    resources.scanline_work = checked_add(resources.scanline_work, work)
        .ok_or("d2raster: scanline work exceeds the int64 domain")?;
    Ok(())
}

pub fn plan_render_resources(
    node: Option<&PreparedNode>,
    dst: Rect,
    patterns: &[std::rc::Rc<crate::paint::PatternTile>],
    max_offscreen_bytes: i64,
) -> Result<RenderResources, String> {
    plan_render_resources_region(node, dst, patterns, max_offscreen_bytes, false)
}

pub fn plan_render_resources_region(
    node: Option<&PreparedNode>,
    dst: Rect,
    patterns: &[std::rc::Rc<crate::paint::PatternTile>],
    max_offscreen_bytes: i64,
    region_filters: bool,
) -> Result<RenderResources, String> {
    let mut planner = RasterizerResourcePlanner {
        counter: Rasterizer::new_counter(0, 0, scanline::max_edges_for_bytes(max_offscreen_bytes)),
        region_filters,
    };
    let mut resources = match node {
        Some(node) => plan_node_resources(node, dst, &mut planner)?,
        None => RenderResources::default(),
    };
    let scene_peak = resources.peak_offscreen_bytes;
    let mut live_pattern_bytes = 0i64;
    let mut pattern_phase_peak = 0i64;
    for pattern in patterns {
        let pattern_resources =
            plan_node_resources(&pattern.root.borrow(), pattern.bounds, &mut planner)?;
        let during_tile = checked_add(live_pattern_bytes, pattern.tile_bytes)
            .ok_or("d2raster: pattern tile storage exceeds the int64 domain")?;
        let during_tile =
            checked_add(during_tile, pattern_resources.peak_offscreen_bytes).ok_or(PEAK_DOMAIN)?;
        pattern_phase_peak = pattern_phase_peak.max(during_tile);
        merge_rasterizer_requirements(&mut resources, &pattern_resources);
        add_scanline_work(&mut resources, pattern_resources.scanline_work)?;
        add_resource_work(&mut resources, pattern_resources.even_odd_clip_work)?;
        live_pattern_bytes = checked_add(live_pattern_bytes, pattern.tile_bytes)
            .ok_or("d2raster: pattern tile storage exceeds the int64 domain")?;
    }
    let scene_peak = checked_add(scene_peak, live_pattern_bytes).ok_or(PEAK_DOMAIN)?;
    resources.peak_offscreen_bytes = pattern_phase_peak.max(scene_peak);
    resources.peak_offscreen_bytes =
        checked_add(resources.peak_offscreen_bytes, resources.rasterizer_bytes)
            .ok_or(PEAK_DOMAIN)?;
    Ok(resources)
}

pub fn uses_effect_layer(node: &PreparedNode) -> bool {
    node.isolated
        || node.opacity < 1.0
        || node.blend != BlendMode::NORMAL
        || !node.filters.is_empty()
        || node.clip.is_some()
        || node.mask.is_some()
}

pub fn plan_node_resources(
    node: &PreparedNode,
    dst: Rect,
    planner: &mut RasterizerResourcePlanner,
) -> Result<RenderResources, String> {
    let mut resources = RenderResources::default();
    if node.opacity == 0.0 {
        return Ok(resources);
    }
    let mut visible_bounds = node.bounds.intersect(dst);
    if visible_bounds.empty() {
        return Ok(resources);
    }
    let uses_filters = !node.filters.is_empty();
    let effect_layer = uses_effect_layer(node);
    if planner.region_filters && effect_layer && !uses_filters {
        visible_bounds = effect_render_bounds(node, dst);
        if visible_bounds.empty() {
            return Ok(resources);
        }
    }
    let mut base_bytes = 0i64;
    let mut final_layer_bytes = 0i64;
    let mut paint_bounds = dst;
    if uses_filters {
        let (filters, input_bounds) = if planner.region_filters {
            crate::filter::regional_filters(&node.filters, node.content_bounds, visible_bounds)?
        } else {
            (node.filters.clone(), node.content_bounds)
        };
        base_bytes = pixel_storage_bytes(input_bounds, 4)
            .map_err(|e| format!("d2raster: filter input layer: {e}"))?;
        let (peak, final_bytes) = crate::filter::plan_filter_resources(&filters, input_bounds)?;
        resources.peak_offscreen_bytes = peak;
        final_layer_bytes = final_bytes;
        paint_bounds = input_bounds;
    } else if effect_layer {
        base_bytes = pixel_storage_bytes(visible_bounds, 4)
            .map_err(|e| format!("d2raster: effect layer: {e}"))?;
        final_layer_bytes = base_bytes;
        resources.peak_offscreen_bytes = base_bytes;
        paint_bounds = visible_bounds;
    }

    let mut primitive_resources = RenderResources::default();
    if let Some(primitive) = &node.primitive {
        if !primitive.bounds.intersect(paint_bounds).empty() {
            primitive_resources = plan_primitive_resources(primitive, paint_bounds, planner)?;
        }
    }
    let with_primitive =
        checked_add(base_bytes, primitive_resources.peak_offscreen_bytes).ok_or(PEAK_DOMAIN)?;
    resources.peak_offscreen_bytes = resources.peak_offscreen_bytes.max(with_primitive);
    merge_rasterizer_requirements(&mut resources, &primitive_resources);
    add_scanline_work(&mut resources, primitive_resources.scanline_work)?;
    add_resource_work(&mut resources, primitive_resources.even_odd_clip_work)?;

    for child in &node.children {
        let child_resources = plan_node_resources(child, paint_bounds, planner)?;
        let with_child =
            checked_add(base_bytes, child_resources.peak_offscreen_bytes).ok_or(PEAK_DOMAIN)?;
        resources.peak_offscreen_bytes = resources.peak_offscreen_bytes.max(with_child);
        merge_rasterizer_requirements(&mut resources, &child_resources);
        add_scanline_work(&mut resources, child_resources.scanline_work)?;
        add_resource_work(&mut resources, child_resources.even_odd_clip_work)?;
    }

    if let Some(clip) = &node.clip {
        if clip.fill_rule == FillRule::EVEN_ODD {
            let clip_bytes = pixel_storage_bytes(visible_bounds, 1)
                .map_err(|e| format!("d2raster: clip mask: {e}"))?;
            let with_clip = checked_add(final_layer_bytes, clip_bytes).ok_or(PEAK_DOMAIN)?;
            resources.peak_offscreen_bytes = resources.peak_offscreen_bytes.max(with_clip);
            let work = even_odd_mask_work(visible_bounds, clip.edges)?;
            add_resource_work(&mut resources, work)?;
        } else if !clip.bounds.intersect(visible_bounds).empty() {
            let origin = if planner.region_filters {
                clip.reference_bounds.min
            } else {
                visible_bounds.min
            };
            let shifted_transform = translate(-(origin.x as f64), -(origin.y as f64));
            planner.record_fill(
                &mut resources,
                visible_bounds,
                &clip.subpaths,
                &shifted_transform,
                "clip",
                origin,
            )?;
        }
    }

    if let Some(mask) = &node.mask {
        let mask_bounds = mask_render_bounds(mask, visible_bounds, planner.region_filters);
        let mask_bytes = pixel_storage_bytes(mask_bounds, 4)
            .map_err(|e| format!("d2raster: scene mask: {e}"))?;
        let mask_resources = plan_node_resources(&mask.root, mask_bounds, planner)?;
        let with_mask = checked_add(final_layer_bytes, mask_bytes)
            .and_then(|v| checked_add(v, mask_resources.peak_offscreen_bytes))
            .ok_or(PEAK_DOMAIN)?;
        resources.peak_offscreen_bytes = resources.peak_offscreen_bytes.max(with_mask);
        merge_rasterizer_requirements(&mut resources, &mask_resources);
        add_scanline_work(&mut resources, mask_resources.scanline_work)?;
        add_resource_work(&mut resources, mask_resources.even_odd_clip_work)?;
    }
    Ok(resources)
}

fn even_odd_mask_work(bounds: Rect, edges: i64) -> Result<i64, String> {
    if bounds.empty() || edges == 0 {
        return Ok(0);
    }
    const DOMAIN: &str = "d2raster: even-odd clip work exceeds the int64 domain";
    let pixels = checked_multiply(bounds.dx(), bounds.dy()).ok_or(DOMAIN)?;
    let work = checked_multiply(
        pixels,
        EVEN_ODD_SAMPLES_PER_AXIS * EVEN_ODD_SAMPLES_PER_AXIS,
    )
    .and_then(|w| checked_multiply(w, edges))
    .ok_or(DOMAIN)?;
    Ok(work)
}

fn plan_primitive_resources(
    primitive: &PreparedPrimitive,
    dst: Rect,
    planner: &mut RasterizerResourcePlanner,
) -> Result<RenderResources, String> {
    let mut resources = RenderResources::default();
    if primitive.image.is_some() {
        return Ok(resources);
    }
    if let Some(vector) = &primitive.vector {
        return plan_node_resources(vector, dst, planner);
    }
    if let Some(fill) = &primitive.fill {
        let bounds = subpath_pixel_bounds(&primitive.subpaths, &primitive.transform, 0.0, dst);
        if primitive.fill_rule == FillRule::EVEN_ODD {
            let bytes = pixel_storage_bytes(bounds, 1)
                .map_err(|e| format!("d2raster: even-odd fill mask: {e}"))?;
            resources.peak_offscreen_bytes = bytes;
            let work = even_odd_mask_work(bounds, primitive.even_odd_edges)?;
            add_resource_work(&mut resources, work)?;
        } else if fill.kind == PaintKind::Solid {
            let origin = if planner.region_filters {
                primitive.reference_bounds.min
            } else {
                dst.min
            };
            let transform = shifted(origin, &primitive.transform);
            planner.record_fill(
                &mut resources,
                dst,
                &primitive.subpaths,
                &transform,
                "solid fill",
                origin,
            )?;
        } else if !bounds.empty() {
            let origin = if planner.region_filters {
                subpath_pixel_bounds(
                    &primitive.subpaths,
                    &primitive.transform,
                    0.0,
                    primitive.reference_bounds,
                )
                .min
            } else {
                bounds.min
            };
            let transform = shifted(origin, &primitive.transform);
            planner.record_fill(
                &mut resources,
                bounds,
                &primitive.subpaths,
                &transform,
                "gradient fill",
                origin,
            )?;
        }
    }
    if let Some(stroke) = &primitive.stroke {
        if stroke.paint.kind == PaintKind::Solid {
            let origin = if planner.region_filters {
                stroke.reference_bounds.min
            } else {
                dst.min
            };
            let transform = shifted(origin, &primitive.transform);
            planner.record_stroke(
                &mut resources,
                dst,
                &primitive.stroke_runs,
                &transform,
                stroke,
                "solid stroke",
                origin,
            )?;
        } else {
            let bounds = painted_stroke_pixel_bounds(
                &primitive.stroke_runs,
                &primitive.transform,
                stroke,
                dst,
            );
            if !bounds.empty() {
                let origin = if planner.region_filters {
                    painted_stroke_pixel_bounds(
                        &primitive.stroke_runs,
                        &primitive.transform,
                        stroke,
                        stroke.reference_bounds,
                    )
                    .min
                } else {
                    bounds.min
                };
                let transform = shifted(origin, &primitive.transform);
                planner.record_stroke(
                    &mut resources,
                    bounds,
                    &primitive.stroke_runs,
                    &transform,
                    stroke,
                    "gradient stroke",
                    origin,
                )?;
            }
        }
    }
    Ok(resources)
}

pub fn merge_rasterizer_requirements(destination: &mut RenderResources, source: &RenderResources) {
    destination.rasterizer_width = destination.rasterizer_width.max(source.rasterizer_width);
    destination.rasterizer_height = destination.rasterizer_height.max(source.rasterizer_height);
    destination.rasterizer_edges = destination.rasterizer_edges.max(source.rasterizer_edges);
    update_rasterizer_bytes(destination);
}

fn record_rasterizer_requirement(resources: &mut RenderResources, bounds: Rect) {
    if bounds.empty() {
        return;
    }
    resources.rasterizer_width = resources.rasterizer_width.max(bounds.dx());
    resources.rasterizer_height = resources.rasterizer_height.max(bounds.dy());
    update_rasterizer_bytes(resources);
}

fn record_rasterizer_edges(resources: &mut RenderResources, edges: i64) {
    resources.rasterizer_edges = resources.rasterizer_edges.max(edges);
    update_rasterizer_bytes(resources);
}

fn update_rasterizer_bytes(resources: &mut RenderResources) {
    resources.rasterizer_bytes = scanline::retained_bytes(
        resources.rasterizer_width,
        resources.rasterizer_height,
        resources.rasterizer_edges,
    )
    .unwrap_or(i64::MAX);
}

pub fn set_rasterizer_reference(
    rasterizer: &mut Rasterizer,
    actual_origin: Pt,
    reference_origin: Pt,
    enabled: bool,
) -> Pt {
    if !enabled {
        return actual_origin;
    }
    rasterizer.set_origin_offset(
        reference_origin.x as f64 - actual_origin.x as f64,
        reference_origin.y as f64 - actual_origin.y as f64,
    );
    reference_origin
}

impl RasterizerResourcePlanner {
    fn record_fill(
        &mut self,
        resources: &mut RenderResources,
        target: Rect,
        paths: &[Subpath],
        transform: &Matrix,
        purpose: &str,
        reference_origin: Pt,
    ) -> Result<(), String> {
        record_rasterizer_requirement(resources, target);
        self.counter.reset(target.dx(), target.dy());
        set_rasterizer_reference(
            &mut self.counter,
            target.min,
            reference_origin,
            self.region_filters,
        );
        for path in paths {
            add_fill_subpath(&mut self.counter, path, transform);
            if let Some(err) = self.counter.err() {
                return Err(rasterizer_plan_error(purpose, err));
            }
        }
        record_rasterizer_edges(resources, self.counter.edge_count());
        let work = self
            .counter
            .work_bound()
            .ok_or_else(|| format!("d2raster: {purpose} scanline work exceeds the int64 domain"))?;
        add_scanline_work(resources, work)
    }

    #[allow(clippy::too_many_arguments)]
    fn record_stroke(
        &mut self,
        resources: &mut RenderResources,
        target: Rect,
        runs: &[StrokeRun],
        transform: &Matrix,
        stroke: &PreparedStroke,
        purpose: &str,
        reference_origin: Pt,
    ) -> Result<(), String> {
        record_rasterizer_requirement(resources, target);
        self.counter.reset(target.dx(), target.dy());
        set_rasterizer_reference(
            &mut self.counter,
            target.min,
            reference_origin,
            self.region_filters,
        );
        for run in runs {
            add_stroke_run(&mut self.counter, run, transform, stroke);
            if let Some(err) = self.counter.err() {
                return Err(rasterizer_plan_error(purpose, err));
            }
        }
        record_rasterizer_edges(resources, self.counter.edge_count());
        let work = self
            .counter
            .work_bound()
            .ok_or_else(|| format!("d2raster: {purpose} scanline work exceeds the int64 domain"))?;
        add_scanline_work(resources, work)
    }
}

fn rasterizer_plan_error(purpose: &str, err: ScanError) -> String {
    if err == ScanError::EdgeLimit {
        return format!(
            "d2raster: {purpose} scanline edges exceed the offscreen storage limit: {err}"
        );
    }
    format!("d2raster: {purpose} geometry: {err}")
}

// ------------------------------------------------------------ effect layers

/// renderEffectNode: paints the subtree into a layer, clips, masks and
/// composites it into dst.
pub fn render_effect_node(
    dst: &mut Rgba<'_>,
    node: &PreparedNode,
    scratch: &mut Scratch,
) -> Result<(), String> {
    if !node.filters.is_empty() {
        return render_filtered_effect_node(dst, node, scratch);
    }
    let mut bounds = node.bounds.intersect(dst.bounds());
    if scratch.region_filters {
        bounds = effect_render_bounds(node, dst.bounds());
    }
    if bounds.empty() {
        return Ok(());
    }
    let (mut layer, layer_bytes) = scratch.offscreen.new_rgba(bounds, "effect layer")?;
    let result = (|| {
        let mut view = layer.view();
        if let Some(primitive) = &node.primitive {
            if !primitive.bounds.intersect(bounds).empty() {
                draw_primitive(&mut view, primitive, scratch)?;
            }
        }
        for child in &node.children {
            render_node(&mut view, child, scratch)?;
        }
        if let Some(clip) = &node.clip {
            apply_clip(&mut view, clip, scratch)?;
        }
        if let Some(mask) = &node.mask {
            apply_mask(&mut view, mask, scratch)?;
        }
        composite_layer(dst, &view, node.opacity, node.blend)
    })();
    scratch.offscreen.release(layer_bytes);
    result
}

fn render_filtered_effect_node(
    dst: &mut Rgba<'_>,
    node: &PreparedNode,
    scratch: &mut Scratch,
) -> Result<(), String> {
    let visible_bounds = node.bounds.intersect(dst.bounds());
    if visible_bounds.empty() || node.content_bounds.empty() {
        return Ok(());
    }
    let (filters, input_bounds) = if scratch.region_filters {
        crate::filter::regional_filters(&node.filters, node.content_bounds, visible_bounds)?
    } else {
        (node.filters.clone(), node.content_bounds)
    };
    let (mut current, mut reservation) = scratch
        .offscreen
        .new_rgba(input_bounds, "filter input layer")?;
    let result = (|| {
        {
            let mut view = current.view();
            if let Some(primitive) = &node.primitive {
                if !primitive.bounds.intersect(input_bounds).empty() {
                    draw_primitive(&mut view, primitive, scratch)?;
                }
            }
            for child in &node.children {
                render_node(&mut view, child, scratch)?;
            }
        }
        for filter in &filters {
            crate::filter::apply_prepared_filter(&mut current, &mut reservation, filter, scratch)?;
        }
        let mut full = current.view();
        let mut layer = if full.bounds() != visible_bounds {
            full.sub_image(visible_bounds)
        } else {
            full.reborrow()
        };
        if let Some(clip) = &node.clip {
            apply_clip(&mut layer, clip, scratch)?;
        }
        if let Some(mask) = &node.mask {
            apply_mask(&mut layer, mask, scratch)?;
        }
        composite_layer(dst, &layer, node.opacity, node.blend)
    })();
    scratch.offscreen.release(reservation);
    result
}

pub fn apply_clip(
    layer: &mut Rgba<'_>,
    clip: &crate::raster::PreparedClip,
    scratch: &mut Scratch,
) -> Result<(), String> {
    if clip.fill_rule != FillRule::EVEN_ODD {
        return apply_non_zero_clip(layer, clip, scratch);
    }
    let bounds = layer.bounds();
    let mask_bounds = Rect::new(0, 0, bounds.dx(), bounds.dy());
    let (mut mask, mask_bytes) = scratch
        .offscreen
        .new_alpha(mask_bounds, "clip Alpha mask")?;
    if !clip.bounds.intersect(bounds).empty() {
        rasterize_even_odd_mask(&mut mask, bounds.min, &clip.subpaths);
    }
    multiply_layer_by_alpha(layer, &mask);
    scratch.offscreen.release(mask_bytes);
    Ok(())
}

fn apply_non_zero_clip(
    layer: &mut Rgba<'_>,
    clip: &crate::raster::PreparedClip,
    scratch: &mut Scratch,
) -> Result<(), String> {
    let bounds = layer.bounds();
    let (width, height) = (bounds.dx(), bounds.dy());
    if width == 0 || height == 0 {
        return Ok(());
    }
    if clip.bounds.intersect(bounds).empty() {
        clear_layer_rows(layer, 0, height);
        return Ok(());
    }
    let region_filters = scratch.region_filters;
    let (rasterizer, budget) = scratch.reset_with_budget(Rect::new(0, 0, width, height));
    let origin = set_rasterizer_reference(
        rasterizer,
        bounds.min,
        clip.reference_bounds.min,
        region_filters,
    );
    let shifted_transform = translate(-(origin.x as f64), -(origin.y as f64));
    for path in &clip.subpaths {
        add_fill_subpath(rasterizer, path, &shifted_transform);
    }
    let mut next_row = 0i64;
    let result = rasterizer.walk_coverage(
        budget,
        |y, min_x, partial: &[f32], difference: &[f32]| -> Result<(), ()> {
            clear_layer_rows(layer, next_row, y);
            let row_offset = layer.pix_offset(bounds.min.x, bounds.min.y + y);
            let row = &mut layer.pix[row_offset..row_offset + width as usize * 4];
            clear_pixel_range(row, 0, min_x as usize);
            let mut winding = 0.0f32;
            let mut index = 0;
            while index < partial.len() {
                if partial[index] == 0.0 && difference[index] == 0.0 {
                    let mut run_end = index + 1;
                    while run_end < partial.len()
                        && partial[run_end] == 0.0
                        && difference[run_end] == 0.0
                    {
                        run_end += 1;
                    }
                    scale_coverage_span(
                        row,
                        min_x as usize + index,
                        min_x as usize + run_end,
                        quantize_coverage(winding),
                    );
                    index = run_end;
                    continue;
                }
                winding += difference[index];
                let coverage = quantize_coverage(partial[index] + winding);
                scale_coverage_span(
                    row,
                    min_x as usize + index,
                    min_x as usize + index + 1,
                    coverage,
                );
                index += 1;
            }
            let max_x = min_x as usize + partial.len();
            clear_pixel_range(row, max_x, width as usize);
            next_row = y + 1;
            Ok(())
        },
    );
    if let Err(err) = result {
        return match err {
            WalkError::Scan(err) => Err(format!("d2raster: clip: {err}")),
            WalkError::Callback(()) => Ok(()),
        };
    }
    clear_layer_rows(layer, next_row, height);
    Ok(())
}

fn clear_layer_rows(layer: &mut Rgba<'_>, first: i64, last: i64) {
    if first >= last {
        return;
    }
    let width = layer.bounds().dx() as usize;
    for y in first..last {
        let row_offset = layer.pix_offset(layer.rect.min.x, layer.rect.min.y + y);
        layer.pix[row_offset..row_offset + width * 4].fill(0);
    }
}

fn clear_pixel_range(row: &mut [u8], first: usize, last: usize) {
    if first < last {
        row[first * 4..last * 4].fill(0);
    }
}

fn scale_coverage_span(row: &mut [u8], first: usize, last: usize, coverage: u8) {
    match coverage {
        0 => clear_pixel_range(row, first, last),
        0xff => {}
        _ => {
            for x in first..last {
                scale_premultiplied(&mut row[x * 4..x * 4 + 4], coverage);
            }
        }
    }
}

/// rasterizeEvenOddMask: 4x4 supersampled even-odd coverage.
pub fn rasterize_even_odd_mask(mask: &mut AlphaBuf, origin: Pt, paths: &[Subpath]) {
    const SAMPLE_COUNT: i64 = EVEN_ODD_SAMPLES_PER_AXIS * EVEN_ODD_SAMPLES_PER_AXIS;
    let samples = EVEN_ODD_SAMPLES_PER_AXIS as f64;
    for y in 0..mask.rect.dy() {
        let row = y as usize * mask.stride;
        let pixel_y = (origin.y + y) as f64;
        let sample_y = [
            pixel_y + 0.5 / samples,
            pixel_y + 1.5 / samples,
            pixel_y + 2.5 / samples,
            pixel_y + 3.5 / samples,
        ];
        for x in 0..mask.rect.dx() {
            let pixel_x = (origin.x + x) as f64;
            let sample_x = [
                pixel_x + 0.5 / samples,
                pixel_x + 1.5 / samples,
                pixel_x + 2.5 / samples,
                pixel_x + 3.5 / samples,
            ];
            let inside = count_even_odd_path_samples(paths, sample_x, sample_y);
            mask.pix[row + x as usize] = ((inside * 255 + SAMPLE_COUNT / 2) / SAMPLE_COUNT) as u8;
        }
    }
}

fn count_even_odd_path_samples(paths: &[Subpath], x: [f64; 4], ys: [f64; 4]) -> i64 {
    let mut inside = 0i64;
    for y in ys {
        let mut parity = 0u8;
        for path in paths {
            if path.points.len() < 2 {
                continue;
            }
            let mut previous = path.points[path.points.len() - 1];
            for current in &path.points {
                if (current.y > y) != (previous.y > y) {
                    let crossing_x = (previous.x - current.x) * (y - current.y)
                        / (previous.y - current.y)
                        + current.x;
                    if x[3] < crossing_x {
                        parity ^= 0b1111;
                    } else if x[2] < crossing_x {
                        parity ^= 0b0111;
                    } else if x[1] < crossing_x {
                        parity ^= 0b0011;
                    } else if x[0] < crossing_x {
                        parity ^= 0b0001;
                    }
                }
                previous = *current;
            }
        }
        inside += parity.count_ones() as i64;
    }
    inside
}

fn multiply_layer_by_alpha(layer: &mut Rgba<'_>, mask: &AlphaBuf) {
    let (width, height) = (layer.bounds().dx() as usize, layer.bounds().dy());
    for y in 0..height {
        let layer_offset = layer.pix_offset(layer.rect.min.x, layer.rect.min.y + y);
        let mask_offset = mask.pix_offset(mask.rect.min.x, mask.rect.min.y + y);
        for x in 0..width {
            let offset = layer_offset + x * 4;
            scale_premultiplied(
                &mut layer.pix[offset..offset + 4],
                mask.pix[mask_offset + x],
            );
        }
    }
}

fn apply_mask(
    layer: &mut Rgba<'_>,
    mask: &PreparedMask,
    scratch: &mut Scratch,
) -> Result<(), String> {
    let bounds = mask_render_bounds(mask, layer.bounds(), scratch.region_filters);
    if bounds.empty() {
        return Ok(());
    }
    let mut layer = if bounds != layer.bounds() {
        layer.sub_image(bounds)
    } else {
        layer.reborrow()
    };
    let (mut mask_layer, mask_bytes) = scratch
        .offscreen
        .new_rgba(bounds, "scene mask RGBA layer")?;
    let result = render_node(&mut mask_layer.view(), &mask.root, scratch);
    if result.is_ok() {
        let mask_ref = mask_layer.as_ref();
        if mask.kind == MaskType::ALPHA {
            multiply_layer_by_rgba(&mut layer, &mask_ref, |pixels, offset| pixels[offset + 3]);
        } else {
            multiply_layer_by_rgba(&mut layer, &mask_ref, rgba_luminance);
        }
    }
    scratch.offscreen.release(mask_bytes);
    result
}

fn multiply_layer_by_rgba(
    layer: &mut Rgba<'_>,
    mask: &crate::pixels::RgbaRef<'_>,
    coverage: impl Fn(&[u8], usize) -> u8,
) {
    let (width, height) = (layer.bounds().dx() as usize, layer.bounds().dy());
    for y in 0..height {
        let layer_offset = layer.pix_offset(layer.rect.min.x, layer.rect.min.y + y);
        let mask_offset = mask.pix_offset(mask.rect.min.x, mask.rect.min.y + y);
        for x in 0..width {
            let value = coverage(mask.pix, mask_offset + x * 4);
            let offset = layer_offset + x * 4;
            scale_premultiplied(&mut layer.pix[offset..offset + 4], value);
        }
    }
}

fn rgba_luminance(pixels: &[u8], offset: usize) -> u8 {
    ((2126 * pixels[offset] as u32
        + 7152 * pixels[offset + 1] as u32
        + 722 * pixels[offset + 2] as u32
        + 5000)
        / 10000) as u8
}

pub fn scale_premultiplied(pixel: &mut [u8], alpha: u8) {
    if alpha == 255 {
        return;
    }
    if alpha == 0 {
        pixel[..4].fill(0);
        return;
    }
    for channel in pixel.iter_mut().take(4) {
        *channel = ((*channel as u32 * alpha as u32 + 127) / 255) as u8;
    }
}

pub fn supported_blend_mode(mode: BlendMode) -> bool {
    matches!(
        mode,
        BlendMode::NORMAL
            | BlendMode::MULTIPLY
            | BlendMode::DARKEN
            | BlendMode::COLOR_BURN
            | BlendMode::OVERLAY
            | BlendMode::LIGHTEN
    )
}

fn supported_prepared_blend_mode(mode: BlendMode) -> bool {
    supported_blend_mode(mode) || mode == PREPARED_COLRV1_SOFT_LIGHT
}

pub fn composite_layer(
    dst: &mut Rgba<'_>,
    layer: &Rgba<'_>,
    opacity: f64,
    mode: BlendMode,
) -> Result<(), String> {
    if mode == BlendMode::NORMAL {
        if opaque_partial_over_prefix_eligible(dst, layer, opacity) {
            composite_layer_over_opaque_partial_prefix(
                dst,
                layer,
                (opacity * 255.0).round() as u32,
            );
            return Ok(());
        }
        composite_layer_over(dst, layer, opacity);
        return Ok(());
    }
    if !supported_prepared_blend_mode(mode) {
        return Err(format!(
            "d2raster: invalid or unsupported blend mode {}",
            mode.0
        ));
    }
    if mode == PREPARED_COLRV1_SOFT_LIGHT {
        composite_colrv1_soft_light_layer(dst, layer, opacity);
        return Ok(());
    }
    if opaque_blend_prefix_eligible(dst, layer, opacity) {
        composite_blend_layer_opaque_prefix(dst, layer, mode);
        return Ok(());
    }
    composite_blend_layer(dst, layer, opacity, mode);
    Ok(())
}

fn first_pixels_opaque(dst: &Rgba<'_>, layer: &Rgba<'_>) -> bool {
    let bounds = layer.bounds().intersect(dst.bounds());
    if bounds.empty() {
        return false;
    }
    let first_source = layer.pix_offset(bounds.min.x, bounds.min.y);
    let first_destination = dst.pix_offset(bounds.min.x, bounds.min.y);
    layer.pix[first_source + 3] == 255 && dst.pix[first_destination + 3] == 255
}

fn opaque_partial_over_prefix_eligible(dst: &Rgba<'_>, layer: &Rgba<'_>, opacity: f64) -> bool {
    let opacity_byte = (opacity * 255.0).round();
    if !(opacity_byte > 0.0 && opacity_byte < 255.0) {
        return false;
    }
    first_pixels_opaque(dst, layer)
}

fn opaque_blend_prefix_eligible(dst: &Rgba<'_>, layer: &Rgba<'_>, opacity: f64) -> bool {
    if opacity == 0.0 || (opacity * 255.0).round() != 255.0 {
        return false;
    }
    first_pixels_opaque(dst, layer)
}

fn composite_colrv1_soft_light_layer(dst: &mut Rgba<'_>, layer: &Rgba<'_>, opacity: f64) {
    let bounds = layer.bounds().intersect(dst.bounds());
    if bounds.empty() || opacity == 0.0 {
        return;
    }
    let opacity_scale = (opacity * 255.0).round() / 255.0;
    for y in bounds.min.y..bounds.max.y {
        let destination_offset = dst.pix_offset(bounds.min.x, y);
        let source_offset = layer.pix_offset(bounds.min.x, y);
        for x in 0..bounds.dx() as usize {
            let si = source_offset + x * 4;
            let di = destination_offset + x * 4;
            let stored_source_alpha = layer.pix[si + 3] as f64 / 255.0;
            let source_alpha = stored_source_alpha * opacity_scale;
            if source_alpha == 0.0 {
                continue;
            }
            let backdrop_alpha = dst.pix[di + 3] as f64 / 255.0;
            // ARM64 effects.go:1432: 1-sourceAlpha re-fuses its product.
            let one_minus_source = go_fmsub(stored_source_alpha, opacity_scale, 1.0);
            let output_alpha = go_fmadd(backdrop_alpha, one_minus_source, source_alpha);
            let output_alpha_byte = rounded_byte(output_alpha * 255.0);
            for channel in 0..3 {
                let source_linear =
                    premultiplied_srgb_byte_to_linear(layer.pix[si + channel], layer.pix[si + 3]);
                let backdrop_linear =
                    premultiplied_srgb_byte_to_linear(dst.pix[di + channel], dst.pix[di + 3]);
                let mixed_linear = soft_light(backdrop_linear, source_linear);
                // ARM64 effects.go:1438–1439.
                let blended = source_alpha * backdrop_alpha * mixed_linear;
                let output = go_fmadd(
                    backdrop_alpha * one_minus_source,
                    backdrop_linear,
                    go_fmadd(
                        source_alpha * (1.0 - backdrop_alpha),
                        source_linear,
                        blended,
                    ),
                );
                let encoded = if output_alpha > 0.0 {
                    linear_to_srgb(output / output_alpha)
                } else {
                    0.0
                };
                let value = rounded_byte(encoded * output_alpha * 255.0).min(output_alpha_byte);
                dst.pix[di + channel] = value;
            }
            dst.pix[di + 3] = output_alpha_byte;
        }
    }
}

fn composite_blend_layer(dst: &mut Rgba<'_>, layer: &Rgba<'_>, opacity: f64, mode: BlendMode) {
    let bounds = layer.bounds().intersect(dst.bounds());
    if bounds.empty() || opacity == 0.0 {
        return;
    }
    let opacity_scale = (opacity * 255.0).round() / 255.0;
    for y in bounds.min.y..bounds.max.y {
        let destination_offset = dst.pix_offset(bounds.min.x, y);
        let source_offset = layer.pix_offset(bounds.min.x, y);
        for x in 0..bounds.dx() as usize {
            let si = source_offset + x * 4;
            let di = destination_offset + x * 4;
            let source_alpha_stored = layer.pix[si + 3] as f64 / 255.0;
            let source_alpha = source_alpha_stored * opacity_scale;
            if source_alpha == 0.0 {
                continue;
            }
            let backdrop_alpha = dst.pix[di + 3] as f64 / 255.0;
            // ARM64 effects.go:1490.
            let one_minus_source = go_fmsub(source_alpha_stored, opacity_scale, 1.0);
            let output_alpha = go_fmadd(backdrop_alpha, one_minus_source, source_alpha);
            let output_alpha_byte = rounded_byte(output_alpha * 255.0);
            let one_minus_backdrop = 1.0 - backdrop_alpha;
            let both = source_alpha * backdrop_alpha;
            for channel in 0..3 {
                let source_premultiplied = layer.pix[si + channel] as f64 / 255.0 * opacity_scale;
                let backdrop_premultiplied = dst.pix[di + channel] as f64 / 255.0;
                let source_color = layer.pix[si + channel] as f64 / layer.pix[si + 3] as f64;
                let backdrop_color = if dst.pix[di + 3] != 0 {
                    dst.pix[di + channel] as f64 / dst.pix[di + 3] as f64
                } else {
                    0.0
                };
                let mixed = blend_component(mode, backdrop_color, source_color);
                // ARM64 effects.go:1501–1503: the first term is fused onto the
                // rounded middle product, then the backdrop term.
                let output = go_fmadd(
                    backdrop_premultiplied,
                    one_minus_source,
                    go_fmadd(source_premultiplied, one_minus_backdrop, both * mixed),
                );
                dst.pix[di + channel] = rounded_byte(output * 255.0).min(output_alpha_byte);
            }
            dst.pix[di + 3] = output_alpha_byte;
        }
    }
}

fn composite_blend_layer_opaque_prefix(dst: &mut Rgba<'_>, layer: &Rgba<'_>, mode: BlendMode) {
    let bounds = layer.bounds().intersect(dst.bounds());
    for y in bounds.min.y..bounds.max.y {
        let destination_offset = dst.pix_offset(bounds.min.x, y);
        let source_offset = layer.pix_offset(bounds.min.x, y);
        let width = bounds.dx() as usize;
        let mut x = 0usize;
        if layer.pix[source_offset + 3] == 0 {
            while x < width && layer.pix[source_offset + x * 4 + 3] == 0 {
                x += 1;
            }
        } else if layer.pix[source_offset + 3] == 255 && dst.pix[destination_offset + 3] == 255 {
            while x < width {
                let si = source_offset + x * 4;
                let di = destination_offset + x * 4;
                if layer.pix[si + 3] != 255 || dst.pix[di + 3] != 255 {
                    break;
                }
                for channel in 0..3 {
                    let backdrop = dst.pix[di + channel];
                    let source = layer.pix[si + channel];
                    dst.pix[di + channel] = match mode {
                        BlendMode::MULTIPLY => {
                            ((backdrop as u32 * source as u32 + 127) / 255) as u8
                        }
                        BlendMode::DARKEN => backdrop.min(source),
                        BlendMode::COLOR_BURN => {
                            let backdrop_color = backdrop as f64 / 255.0;
                            let source_color = source as f64 / 255.0;
                            if backdrop_color >= 1.0 {
                                255
                            } else if source_color <= 0.0 {
                                0
                            } else {
                                rounded_byte(
                                    (1.0 - go_min(1.0, (1.0 - backdrop_color) / source_color))
                                        * 255.0,
                                )
                            }
                        }
                        BlendMode::OVERLAY => overlay_opaque_byte(backdrop, source),
                        BlendMode::LIGHTEN => backdrop.max(source),
                        _ => backdrop,
                    };
                }
                dst.pix[di + 3] = 255;
                x += 1;
            }
        }
        while x < width {
            let si = source_offset + x * 4;
            let di = destination_offset + x * 4;
            x += 1;
            let source_alpha_stored = layer.pix[si + 3] as f64 / 255.0;
            if source_alpha_stored == 0.0 {
                continue;
            }
            let backdrop_alpha = dst.pix[di + 3] as f64 / 255.0;
            let one_minus_source = 1.0 - source_alpha_stored;
            // ARM64 effects.go:1602.
            let output_alpha = go_fmadd(backdrop_alpha, one_minus_source, source_alpha_stored);
            let output_alpha_byte = rounded_byte(output_alpha * 255.0);
            let one_minus_backdrop = 1.0 - backdrop_alpha;
            let both = source_alpha_stored * backdrop_alpha;
            for channel in 0..3 {
                let source_premultiplied = layer.pix[si + channel] as f64 / 255.0;
                let backdrop_premultiplied = dst.pix[di + channel] as f64 / 255.0;
                let source_color = layer.pix[si + channel] as f64 / layer.pix[si + 3] as f64;
                let backdrop_color = if dst.pix[di + 3] != 0 {
                    dst.pix[di + channel] as f64 / dst.pix[di + 3] as f64
                } else {
                    0.0
                };
                let mixed = blend_component(mode, backdrop_color, source_color);
                // ARM64 effects.go:1613–1615 (same shape as compositeBlendLayer).
                let output = go_fmadd(
                    backdrop_premultiplied,
                    one_minus_source,
                    go_fmadd(source_premultiplied, one_minus_backdrop, both * mixed),
                );
                dst.pix[di + channel] = rounded_byte(output * 255.0).min(output_alpha_byte);
            }
            dst.pix[di + 3] = output_alpha_byte;
        }
    }
}

fn overlay_opaque_byte(backdrop: u8, source: u8) -> u8 {
    let (b, s) = (backdrop as u32, source as u32);
    if b <= 127 {
        return ((2 * b * s + 127) / 255) as u8;
    }
    (255 - (2 * (255 - b) * (255 - s) + 127) / 255) as u8
}

fn blend_component(mode: BlendMode, backdrop: f64, source: f64) -> f64 {
    match mode {
        BlendMode::MULTIPLY => backdrop * source,
        BlendMode::DARKEN => go_min(backdrop, source),
        BlendMode::COLOR_BURN => {
            if backdrop >= 1.0 {
                return 1.0;
            }
            if source <= 0.0 {
                return 0.0;
            }
            1.0 - go_min(1.0, (1.0 - backdrop) / source)
        }
        BlendMode::OVERLAY => {
            if backdrop <= 0.5 {
                return 2.0 * backdrop * source;
            }
            // ARM64 effects.go:1658.
            go_fmsub(2.0 * (1.0 - backdrop), 1.0 - source, 1.0)
        }
        BlendMode::LIGHTEN => go_max(backdrop, source),
        PREPARED_COLRV1_SOFT_LIGHT => soft_light(backdrop, source),
        _ => source,
    }
}

fn soft_light(backdrop: f64, source: f64) -> f64 {
    if source <= 0.5 {
        // ARM64 effects.go:1670.
        return go_fmsub((1.0 - 2.0 * source) * backdrop, 1.0 - backdrop, backdrop);
    }
    let mut d = backdrop.sqrt();
    if backdrop <= 0.25 {
        // ARM64 effects.go:1674: FNMSUBD then FMADDD.
        d = go_fmadd(go_fnmsub(16.0, backdrop, 12.0), backdrop, 4.0) * backdrop;
    }
    // ARM64 effects.go:1676.
    go_fmadd(2.0 * source - 1.0, d - backdrop, backdrop)
}

fn composite_layer_over_opaque_partial_prefix(
    dst: &mut Rgba<'_>,
    layer: &Rgba<'_>,
    opacity_byte: u32,
) {
    let bounds = layer.bounds().intersect(dst.bounds());
    let mul255 = |left: u32, right: u32| (left * right + 127) / 255;
    for y in bounds.min.y..bounds.max.y {
        let destination_offset = dst.pix_offset(bounds.min.x, y);
        let source_offset = layer.pix_offset(bounds.min.x, y);
        let width = bounds.dx() as usize;
        let mut x = 0usize;
        if layer.pix[source_offset + 3] == 255 && dst.pix[destination_offset + 3] == 255 {
            while x < width {
                let si = source_offset + x * 4;
                let di = destination_offset + x * 4;
                if layer.pix[si + 3] != 255 || dst.pix[di + 3] != 255 {
                    break;
                }
                for channel in 0..3 {
                    let scaled_source = (layer.pix[si + channel] as u32 * opacity_byte + 127) / 255;
                    let inverse_alpha = 255 - opacity_byte;
                    let value =
                        scaled_source + (dst.pix[di + channel] as u32 * inverse_alpha + 127) / 255;
                    dst.pix[di + channel] = value.min(255) as u8;
                }
                dst.pix[di + 3] = 255;
                x += 1;
            }
        }
        while x < width {
            let si = source_offset + x * 4;
            let di = destination_offset + x * 4;
            x += 1;
            let source_alpha = mul255(layer.pix[si + 3] as u32, opacity_byte);
            if source_alpha == 0 {
                continue;
            }
            let inverse_alpha = 255 - source_alpha;
            for channel in 0..3 {
                let source = mul255(layer.pix[si + channel] as u32, opacity_byte);
                let value = source + mul255(dst.pix[di + channel] as u32, inverse_alpha);
                dst.pix[di + channel] = value.min(255) as u8;
            }
            let alpha = source_alpha + mul255(dst.pix[di + 3] as u32, inverse_alpha);
            dst.pix[di + 3] = alpha.min(255) as u8;
        }
    }
}

/// compositeLayerOver: deterministic integer source-over with group
/// opacity. Go's uniform-block probing produces the same bytes.
pub fn composite_layer_over(dst: &mut Rgba<'_>, layer: &Rgba<'_>, opacity: f64) {
    let bounds = layer.bounds().intersect(dst.bounds());
    if bounds.empty() || opacity == 0.0 {
        return;
    }
    let opacity_byte = (opacity * 255.0).round() as u32;
    let mul255 = |left: u32, right: u32| (left * right + 127) / 255;
    for y in bounds.min.y..bounds.max.y {
        let destination_offset = dst.pix_offset(bounds.min.x, y);
        let source_offset = layer.pix_offset(bounds.min.x, y);
        for x in 0..bounds.dx() as usize {
            let si = source_offset + x * 4;
            let di = destination_offset + x * 4;
            if opacity_byte == 0xff {
                let source_alpha = layer.pix[si + 3] as u32;
                if source_alpha == 0 {
                    continue;
                }
                if source_alpha == 0xff {
                    dst.pix[di] = layer.pix[si];
                    dst.pix[di + 1] = layer.pix[si + 1];
                    dst.pix[di + 2] = layer.pix[si + 2];
                    dst.pix[di + 3] = 0xff;
                    continue;
                }
                let inverse_alpha = 255 - source_alpha;
                for channel in 0..3 {
                    let value = layer.pix[si + channel] as u32
                        + mul255(dst.pix[di + channel] as u32, inverse_alpha);
                    dst.pix[di + channel] = value.min(255) as u8;
                }
                let alpha = source_alpha + mul255(dst.pix[di + 3] as u32, inverse_alpha);
                dst.pix[di + 3] = alpha.min(255) as u8;
                continue;
            }
            let source_alpha = mul255(layer.pix[si + 3] as u32, opacity_byte);
            if source_alpha == 0 {
                continue;
            }
            let inverse_alpha = 255 - source_alpha;
            for channel in 0..3 {
                let source = mul255(layer.pix[si + channel] as u32, opacity_byte);
                let value = source + mul255(dst.pix[di + channel] as u32, inverse_alpha);
                dst.pix[di + channel] = value.min(255) as u8;
            }
            let alpha = source_alpha + mul255(dst.pix[di + 3] as u32, inverse_alpha);
            dst.pix[di + 3] = alpha.min(255) as u8;
        }
    }
}
