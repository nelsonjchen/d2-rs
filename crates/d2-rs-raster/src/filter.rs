// SPDX-License-Identifier: MPL-2.0
//! d2raster/filter.go: Gaussian blur (three box passes per axis) and drop
//! shadows. Go's "expanded" box-blur kernels and integer-offset shadow
//! kernels are specializations that produce the same bytes as the general
//! kernels ported here (see SOURCE.md).

use std::collections::BTreeMap;

use d2_rs_scene::{go_max, go_min, DropShadow, Filter, Matrix, Nrgba, Point};

use crate::effects::{checked_add, composite_layer_over, pixel_storage_bytes};
use crate::fma::{finite, finite_point, go_fmadd, gomath, mvector};
use crate::paint::rounded_byte;
use crate::pixels::{union_rect, AlphaBuf, Rect, RgbaBuf, RgbaRef};
use crate::raster::Scratch;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    GaussianBlur,
    DropShadow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlurAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug)]
pub struct BlurPass {
    pub axis: BlurAxis,
    pub radius: i64,
    pub bounds: Rect,
}

#[derive(Clone, Debug)]
pub struct PreparedFilter {
    pub kind: FilterKind,
    pub passes: Vec<BlurPass>,
    pub output: Rect,
    pub offset_x: f64,
    pub offset_y: f64,
    pub shadow_color: Nrgba,
}

#[derive(Clone, Copy, Debug)]
pub struct NormalizedFilter {
    kind: FilterKind,
    sigma_x: f64,
    sigma_y: f64,
    offset_x: f64,
    offset_y: f64,
    shadow_color: Nrgba,
    source_index: usize,
}

pub fn normalize_node_filters(node_id: &str, filters: &[Filter], animated: &BTreeMap<i64, DropShadow>) -> Result<Vec<NormalizedFilter>, String> {
    for index in animated.keys() {
        if *index < 0 || *index >= filters.len() as i64 {
            return Err(format!(
                "d2raster: node {node_id:?} drop-shadow animation target index {index} is outside {} filters",
                filters.len()
            ));
        }
    }
    let mut result = Vec::with_capacity(filters.len());
    for (index, raw) in filters.iter().enumerate() {
        let mut filter = NormalizedFilter {
            kind: FilterKind::GaussianBlur,
            sigma_x: 0.0,
            sigma_y: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            shadow_color: Nrgba::default(),
            source_index: index,
        };
        match raw {
            Filter::GaussianBlur { sigma_x, sigma_y } => {
                filter.sigma_x = *sigma_x;
                filter.sigma_y = *sigma_y;
            }
            Filter::DropShadow(value) => {
                filter.kind = FilterKind::DropShadow;
                filter.sigma_x = value.sigma_x;
                filter.sigma_y = value.sigma_y;
                filter.offset_x = value.offset_x;
                filter.offset_y = value.offset_y;
                filter.shadow_color = value.color;
            }
        }
        validate_normalized_filter(node_id, index, &filter)?;
        if let Some(over) = animated.get(&(index as i64)) {
            if filter.kind != FilterKind::DropShadow {
                return Err(format!(
                    "d2raster: node {node_id:?} drop-shadow animation target index {index} does not identify a drop shadow"
                ));
            }
            filter.sigma_x = over.sigma_x;
            filter.sigma_y = over.sigma_y;
            filter.offset_x = over.offset_x;
            filter.offset_y = over.offset_y;
            filter.shadow_color = over.color;
            validate_normalized_filter(node_id, index, &filter)?;
        }
        result.push(filter);
    }
    Ok(result)
}

fn validate_normalized_filter(node_id: &str, index: usize, filter: &NormalizedFilter) -> Result<(), String> {
    if filter.sigma_x < 0.0 || filter.sigma_y < 0.0 || !finite(filter.sigma_x) || !finite(filter.sigma_y) {
        return Err(format!("d2raster: node {node_id:?} filter {index} has invalid Gaussian deviation"));
    }
    if filter.kind == FilterKind::DropShadow && (!finite(filter.offset_x) || !finite(filter.offset_y)) {
        return Err(format!("d2raster: node {node_id:?} filter {index} has an invalid drop-shadow offset"));
    }
    Ok(())
}

pub fn prepare_filters(node_id: &str, filters: &[NormalizedFilter], transform: &Matrix, input: Rect) -> Result<(Vec<PreparedFilter>, Rect), String> {
    let mut current = input;
    if current.empty() {
        return Ok((Vec::new(), current));
    }
    let mut result = Vec::with_capacity(filters.len());
    for filter in filters {
        if filter.kind == FilterKind::DropShadow && filter.shadow_color.a == 0 {
            continue;
        }
        let source = filter.source_index;
        let sigma_x = gomath::hypot(transform.a * filter.sigma_x, transform.c * filter.sigma_y);
        let sigma_y = gomath::hypot(transform.b * filter.sigma_x, transform.d * filter.sigma_y);
        if !finite(sigma_x) || !finite(sigma_y) {
            return Err(format!("d2raster: node {node_id:?} filter {source} has non-finite transformed deviation"));
        }
        let radius_x = blur_support_radius(sigma_x).map_err(|e| format!("d2raster: node {node_id:?} filter {source} horizontal deviation: {e}"))?;
        let radius_y = blur_support_radius(sigma_y).map_err(|e| format!("d2raster: node {node_id:?} filter {source} vertical deviation: {e}"))?;
        let mut prepared = PreparedFilter {
            kind: filter.kind,
            passes: Vec::new(),
            output: Rect::default(),
            offset_x: 0.0,
            offset_y: 0.0,
            shadow_color: filter.shadow_color,
        };
        let wrap = |e: String| format!("d2raster: node {node_id:?} filter {source}: {e}");
        match filter.kind {
            FilterKind::GaussianBlur => {
                if current.empty() || radius_x == 0 && radius_y == 0 {
                    continue;
                }
                let (passes, output) = prepare_blur_passes(current, radius_x, radius_y).map_err(wrap)?;
                prepared.passes = passes;
                prepared.output = output;
                current = output;
            }
            FilterKind::DropShadow => {
                let offset = mvector(transform, Point::new(filter.offset_x, filter.offset_y));
                if !finite_point(offset) {
                    return Err(format!("d2raster: node {node_id:?} filter {source} has a non-finite transformed shadow offset"));
                }
                prepared.offset_x = offset.x;
                prepared.offset_y = offset.y;
                let (passes, blurred) = prepare_blur_passes(current, radius_x, radius_y).map_err(wrap)?;
                prepared.passes = passes;
                let shadow_bounds = translated_filter_bounds_unbounded(blurred, offset.x, offset.y).map_err(wrap)?;
                prepared.output = union_filter_bounds(current, shadow_bounds).map_err(wrap)?;
                current = prepared.output;
            }
        }
        result.push(prepared);
    }
    Ok((result, current))
}

const MAX_BLUR_SUPPORT_RADIUS: f64 = (1u64 << 30) as f64;

fn blur_support_radius(sigma: f64) -> Result<i64, String> {
    if sigma == 0.0 {
        return Ok(0);
    }
    let expansion = (3.0 * sigma).ceil();
    if !finite(expansion) || expansion > MAX_BLUR_SUPPORT_RADIUS {
        return Err("three-sigma support exceeds the bounded raster domain".into());
    }
    Ok(expansion as i64)
}

fn prepare_blur_passes(input: Rect, radius_x: i64, radius_y: i64) -> Result<(Vec<BlurPass>, Rect), String> {
    let x_radii = split_blur_radius(radius_x);
    let y_radii = split_blur_radius(radius_y);
    let mut current = input;
    let mut passes = Vec::with_capacity(6);
    for index in 0..3 {
        if x_radii[index] != 0 {
            current = expand_filter_bounds(current, x_radii[index], 0)?;
            passes.push(BlurPass { axis: BlurAxis::Horizontal, radius: x_radii[index], bounds: current });
        }
        if y_radii[index] != 0 {
            current = expand_filter_bounds(current, 0, y_radii[index])?;
            passes.push(BlurPass { axis: BlurAxis::Vertical, radius: y_radii[index], bounds: current });
        }
    }
    Ok((passes, current))
}

fn split_blur_radius(radius: i64) -> [i64; 3] {
    let mut result = [radius / 3; 3];
    for slot in result.iter_mut().take((radius % 3) as usize) {
        *slot += 1;
    }
    result
}

fn expand_filter_bounds(bounds: Rect, x: i64, y: i64) -> Result<Rect, String> {
    if bounds.empty() {
        return Ok(Rect::default());
    }
    let max_platform = i64::MAX;
    let min_platform = i64::MIN;
    if bounds.min.x < min_platform + x || bounds.min.y < min_platform + y || bounds.max.x > max_platform - x || bounds.max.y > max_platform - y {
        return Err("filter bounds exceed the platform integer domain".into());
    }
    let (min_x, min_y) = (bounds.min.x - x, bounds.min.y - y);
    let (max_x, max_y) = (bounds.max.x + x, bounds.max.y + y);
    if (max_x as u64).wrapping_sub(min_x as u64) > max_platform as u64 || (max_y as u64).wrapping_sub(min_y as u64) > max_platform as u64 {
        return Err("filter bounds exceed the platform integer domain".into());
    }
    Ok(Rect::new(min_x, min_y, max_x, max_y))
}

fn translated_filter_bounds_unbounded(bounds: Rect, offset_x: f64, offset_y: f64) -> Result<Rect, String> {
    if bounds.empty() {
        return Ok(Rect::default());
    }
    let min_x = (bounds.min.x as f64 + offset_x).floor();
    let min_y = (bounds.min.y as f64 + offset_y).floor();
    let max_x = (bounds.max.x as f64 + offset_x).ceil();
    let max_y = (bounds.max.y as f64 + offset_y).ceil();
    let max_coordinate = next_after_toward_zero(i64::MAX as f64);
    let min_platform = i64::MIN as f64;
    if !finite(min_x)
        || !finite(min_y)
        || !finite(max_x)
        || !finite(max_y)
        || min_x < min_platform
        || min_y < min_platform
        || max_x > max_coordinate
        || max_y > max_coordinate
        || max_x <= min_x
        || max_y <= min_y
    {
        return Err("translated filter bounds exceed the platform integer domain".into());
    }
    let result = Rect::new(min_x as i64, min_y as i64, max_x as i64, max_y as i64);
    if (result.max.x as u64).wrapping_sub(result.min.x as u64) > i64::MAX as u64
        || (result.max.y as u64).wrapping_sub(result.min.y as u64) > i64::MAX as u64
    {
        return Err("translated filter bounds exceed the platform integer domain".into());
    }
    Ok(result)
}

fn next_after_toward_zero(value: f64) -> f64 {
    f64::from_bits(value.to_bits() - 1)
}

fn union_filter_bounds(left: Rect, right: Rect) -> Result<Rect, String> {
    let result = union_rect(left, right);
    if result.empty() {
        return Ok(result);
    }
    if (result.max.x as u64).wrapping_sub(result.min.x as u64) > i64::MAX as u64
        || (result.max.y as u64).wrapping_sub(result.min.y as u64) > i64::MAX as u64
    {
        return Err("filter bounds exceed the platform integer domain".into());
    }
    Ok(result)
}

/// regionalFilters: only the region each pass's successor consumes.
pub fn regional_filters(filters: &[PreparedFilter], input: Rect, destination: Rect) -> Result<(Vec<PreparedFilter>, Rect), String> {
    let mut result = filters.to_vec();
    let mut needed = destination;
    for index in (0..filters.len()).rev() {
        let mut filter = filters[index].clone();
        let previous = if index > 0 { filters[index - 1].output } else { input };
        filter.output = needed.intersect(filter.output);
        let mut blurred_needed = filter.output;
        if filter.kind == FilterKind::DropShadow {
            let blurred_bounds = filter.passes.last().map_or(previous, |p| p.bounds);
            blurred_needed = shadow_sample_bounds(filter.output, blurred_bounds, filter.offset_x, filter.offset_y);
        }
        for pass_index in (0..filter.passes.len()).rev() {
            let pass_bounds = blurred_needed.intersect(filter.passes[pass_index].bounds);
            filter.passes[pass_index].bounds = pass_bounds;
            let previous_pass = if pass_index > 0 { filters[index].passes[pass_index - 1].bounds } else { previous };
            let (x, y) = match filter.passes[pass_index].axis {
                BlurAxis::Horizontal => (filter.passes[pass_index].radius, 0),
                BlurAxis::Vertical => (0, filter.passes[pass_index].radius),
            };
            blurred_needed = match expand_filter_bounds(pass_bounds, x, y) {
                Ok(expanded) => expanded.intersect(previous_pass),
                Err(_) => previous_pass,
            };
        }
        needed = blurred_needed.intersect(previous);
        if filter.kind == FilterKind::DropShadow {
            needed = union_rect(needed, filter.output.intersect(previous));
        }
        result[index] = filter;
    }
    Ok((result, needed.intersect(input)))
}

fn rectangle_within_exact_float_integer_domain(bounds: Rect) -> bool {
    let max = 1i64 << 53;
    [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y].iter().all(|v| *v >= -max && *v <= max)
}

fn shadow_sample_bounds(destination: Rect, source: Rect, offset_x: f64, offset_y: f64) -> Rect {
    if destination.empty() || source.empty() {
        return Rect::default();
    }
    let max_exact_integer = (1u64 << 53) as f64;
    if !rectangle_within_exact_float_integer_domain(destination)
        || !rectangle_within_exact_float_integer_domain(source)
        || !finite(offset_x)
        || !finite(offset_y)
        || offset_x.abs() > max_exact_integer
        || offset_y.abs() > max_exact_integer
    {
        return source;
    }
    let padding = if offset_x == offset_x.trunc() && offset_y == offset_y.trunc() { 1.0 } else { 2.0 };
    let min_x = go_max((destination.min.x as f64 - offset_x).floor(), source.min.x as f64);
    let min_y = go_max((destination.min.y as f64 - offset_y).floor(), source.min.y as f64);
    let max_x = go_min(((destination.max.x - 1) as f64 - offset_x).floor() + padding, source.max.x as f64);
    let max_y = go_min(((destination.max.y - 1) as f64 - offset_y).floor() + padding, source.max.y as f64);
    if min_x >= max_x || min_y >= max_y {
        return Rect::default();
    }
    Rect::new(min_x as i64, min_y as i64, max_x as i64, max_y as i64)
}

/// applyPreparedFilter replaces `current` with the filter's output layer.
pub fn apply_prepared_filter(current: &mut RgbaBuf, reservation: &mut i64, filter: &PreparedFilter, scratch: &mut Scratch) -> Result<(), String> {
    match filter.kind {
        FilterKind::GaussianBlur => {
            for (index, pass) in filter.passes.iter().enumerate() {
                let (mut output, output_bytes) = scratch.offscreen.new_rgba(pass.bounds, "Gaussian blur pass")?;
                box_blur_rgba(&mut output, current, pass).map_err(|e| format!("d2raster: Gaussian blur pass {index}: {e}"))?;
                scratch.offscreen.release(*reservation);
                *current = output;
                *reservation = output_bytes;
            }
            Ok(())
        }
        FilterKind::DropShadow => {
            let mut alpha: Option<(AlphaBuf, i64)> = None;
            for pass in &filter.passes {
                let (mut next, next_bytes) = match scratch.offscreen.new_alpha(pass.bounds, "drop-shadow blur pass") {
                    Ok(value) => value,
                    Err(err) => {
                        if let Some((_, bytes)) = alpha {
                            scratch.offscreen.release(bytes);
                        }
                        return Err(err);
                    }
                };
                match &alpha {
                    None => box_blur_alpha_pixels(&mut next, &current.pix, current.stride, current.rect, 4, 3, pass),
                    Some((source, _)) => box_blur_alpha_pixels(&mut next, &source.pix, source.stride, source.rect, 1, 0, pass),
                }
                if let Some((_, bytes)) = alpha.take() {
                    scratch.offscreen.release(bytes);
                }
                alpha = Some((next, next_bytes));
            }
            let (mut output, output_bytes) = match scratch.offscreen.new_rgba(filter.output, "drop-shadow output layer") {
                Ok(value) => value,
                Err(err) => {
                    if let Some((_, bytes)) = alpha {
                        scratch.offscreen.release(bytes);
                    }
                    return Err(err);
                }
            };
            paint_drop_shadow(&mut output, current.as_ref(), alpha.as_ref().map(|(a, _)| a), filter.offset_x, filter.offset_y, filter.shadow_color);
            composite_layer_over(&mut output.view(), &current.view(), 1.0);
            if let Some((_, bytes)) = alpha {
                scratch.offscreen.release(bytes);
            }
            scratch.offscreen.release(*reservation);
            *current = output;
            *reservation = output_bytes;
            Ok(())
        }
    }
}

fn box_blur_rgba(destination: &mut RgbaBuf, source: &RgbaBuf, pass: &BlurPass) -> Result<(), String> {
    for channel in 0..4 {
        box_blur_channel(&mut destination.pix, destination.stride, destination.rect, 4, channel, &source.pix, source.stride, source.rect, 4, channel, pass);
    }
    Ok(())
}

fn box_blur_alpha_pixels(destination: &mut AlphaBuf, source_pixels: &[u8], source_stride: usize, source_bounds: Rect, source_step: usize, source_channel: usize, pass: &BlurPass) {
    let (stride, rect) = (destination.stride, destination.rect);
    box_blur_channel(&mut destination.pix, stride, rect, 1, 0, source_pixels, source_stride, source_bounds, source_step, source_channel, pass);
}

/// boxBlurRGBAGeneral / boxBlurAlphaPixelsGeneral for one channel: a moving
/// window sum over transparent-outside source pixels.
#[allow(clippy::too_many_arguments)]
fn box_blur_channel(
    destination: &mut [u8],
    destination_stride: usize,
    destination_bounds: Rect,
    destination_step: usize,
    destination_channel: usize,
    source: &[u8],
    source_stride: usize,
    source_bounds: Rect,
    source_step: usize,
    source_channel: usize,
    pass: &BlurPass,
) {
    let window = pass.radius * 2 + 1;
    let radius = pass.radius;
    let at = |x: i64, y: i64| -> i64 {
        source[(y - source_bounds.min.y) as usize * source_stride + (x - source_bounds.min.x) as usize * source_step + source_channel] as i64
    };
    match pass.axis {
        BlurAxis::Horizontal => {
            for y in destination_bounds.min.y..destination_bounds.max.y {
                let row_in_source = y >= source_bounds.min.y && y < source_bounds.max.y;
                let mut sum = 0i64;
                let x0 = destination_bounds.min.x;
                if row_in_source {
                    let sample_min = (x0 - radius).max(source_bounds.min.x);
                    let sample_max = (x0 + radius).min(source_bounds.max.x - 1);
                    for sample_x in sample_min..=sample_max {
                        sum += at(sample_x, y);
                    }
                }
                let destination_offset = (y - destination_bounds.min.y) as usize * destination_stride;
                for x in destination_bounds.min.x..destination_bounds.max.x {
                    destination[destination_offset + (x - destination_bounds.min.x) as usize * destination_step + destination_channel] =
                        ((sum + window / 2) / window) as u8;
                    let (remove_x, add_x) = (x - radius, x + radius + 1);
                    if row_in_source && remove_x >= source_bounds.min.x && remove_x < source_bounds.max.x {
                        sum -= at(remove_x, y);
                    }
                    if row_in_source && add_x >= source_bounds.min.x && add_x < source_bounds.max.x {
                        sum += at(add_x, y);
                    }
                }
            }
        }
        BlurAxis::Vertical => {
            for x in destination_bounds.min.x..destination_bounds.max.x {
                let column_in_source = x >= source_bounds.min.x && x < source_bounds.max.x;
                let mut sum = 0i64;
                let y0 = destination_bounds.min.y;
                if column_in_source {
                    let sample_min = (y0 - radius).max(source_bounds.min.y);
                    let sample_max = (y0 + radius).min(source_bounds.max.y - 1);
                    for sample_y in sample_min..=sample_max {
                        sum += at(x, sample_y);
                    }
                }
                let destination_column = (x - destination_bounds.min.x) as usize * destination_step + destination_channel;
                for y in destination_bounds.min.y..destination_bounds.max.y {
                    destination[(y - destination_bounds.min.y) as usize * destination_stride + destination_column] = ((sum + window / 2) / window) as u8;
                    let (remove_y, add_y) = (y - radius, y + radius + 1);
                    if column_in_source && remove_y >= source_bounds.min.y && remove_y < source_bounds.max.y {
                        sum -= at(x, remove_y);
                    }
                    if column_in_source && add_y >= source_bounds.min.y && add_y < source_bounds.max.y {
                        sum += at(x, add_y);
                    }
                }
            }
        }
    }
}

fn paint_drop_shadow(destination: &mut RgbaBuf, source: RgbaRef<'_>, blurred: Option<&AlphaBuf>, offset_x: f64, offset_y: f64, shadow: Nrgba) {
    let bounds = destination.rect;
    for y in bounds.min.y..bounds.max.y {
        let sample_y = y as f64 - offset_y;
        for x in bounds.min.x..bounds.max.x {
            let alpha = match blurred {
                None => sample_shadow_alpha(source.pix, source.stride, source.rect, 4, 3, x as f64 - offset_x, sample_y),
                Some(blurred) => sample_shadow_alpha(&blurred.pix, blurred.stride, blurred.rect, 1, 0, x as f64 - offset_x, sample_y),
            };
            if alpha == 0 {
                continue;
            }
            let pixel = premultiplied_shadow_pixel(shadow, alpha);
            let offset = (y - bounds.min.y) as usize * destination.stride + (x - bounds.min.x) as usize * 4;
            destination.pix[offset..offset + 4].copy_from_slice(&pixel);
        }
    }
}

fn premultiplied_shadow_pixel(shadow: Nrgba, alpha: u8) -> [u8; 4] {
    let shadow_alpha = ((alpha as u32 * shadow.a as u32 + 127) / 255) as u8;
    [
        ((shadow.r as u32 * shadow_alpha as u32 + 127) / 255) as u8,
        ((shadow.g as u32 * shadow_alpha as u32 + 127) / 255) as u8,
        ((shadow.b as u32 * shadow_alpha as u32 + 127) / 255) as u8,
        shadow_alpha,
    ]
}

/// sampleShadowAlphaRGBA / sampleShadowAlphaImage: bilinear alpha sampling.
fn sample_shadow_alpha(pixels: &[u8], stride: usize, bounds: Rect, step: usize, channel: usize, x: f64, y: f64) -> u8 {
    if !finite(x) || !finite(y) || x < bounds.min.x as f64 - 1.0 || x >= bounds.max.x as f64 || y < bounds.min.y as f64 - 1.0 || y >= bounds.max.y as f64 {
        return 0;
    }
    let (x0, y0) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x0 as f64, y - y0 as f64);
    let at = |px: i64, py: i64| -> u8 {
        if !bounds.contains(crate::pixels::Pt::new(px, py)) {
            return 0;
        }
        pixels[(py - bounds.min.y) as usize * stride + (px - bounds.min.x) as usize * step + channel]
    };
    if fx == 0.0 && fy == 0.0 {
        return at(x0, y0);
    }
    interpolate_shadow_alpha(at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1), fx, fy)
}

fn interpolate_shadow_alpha(a00: u8, a10: u8, a01: u8, a11: u8, fx: f64, fy: f64) -> u8 {
    // ARM64 filter.go:1195–1197.
    let top = go_fmadd(a10 as f64 - a00 as f64, fx, a00 as f64);
    let bottom = go_fmadd(a11 as f64 - a01 as f64, fx, a01 as f64);
    rounded_byte(go_fmadd(bottom - top, fy, top))
}

pub fn plan_filter_resources(filters: &[PreparedFilter], input: Rect) -> Result<(i64, i64), String> {
    const PEAK: &str = "d2raster: peak offscreen pixel storage exceeds the int64 domain";
    let mut current_bytes = pixel_storage_bytes(input, 4).map_err(|e| format!("d2raster: filter input layer: {e}"))?;
    let mut peak = current_bytes;
    for filter in filters {
        match filter.kind {
            FilterKind::GaussianBlur => {
                for pass in &filter.passes {
                    let next_bytes = pixel_storage_bytes(pass.bounds, 4).map_err(|e| format!("d2raster: Gaussian blur pass: {e}"))?;
                    let live = checked_add(current_bytes, next_bytes).ok_or(PEAK)?;
                    peak = peak.max(live);
                    current_bytes = next_bytes;
                }
            }
            FilterKind::DropShadow => {
                let mut alpha_bytes = 0i64;
                for pass in &filter.passes {
                    let next_bytes = pixel_storage_bytes(pass.bounds, 1).map_err(|e| format!("d2raster: drop-shadow blur pass: {e}"))?;
                    let live = checked_add(current_bytes, alpha_bytes).and_then(|v| checked_add(v, next_bytes)).ok_or(PEAK)?;
                    peak = peak.max(live);
                    alpha_bytes = next_bytes;
                }
                let output_bytes = pixel_storage_bytes(filter.output, 4).map_err(|e| format!("d2raster: drop-shadow output layer: {e}"))?;
                let live = checked_add(current_bytes, alpha_bytes).and_then(|v| checked_add(v, output_bytes)).ok_or(PEAK)?;
                peak = peak.max(live);
                current_bytes = output_bytes;
            }
        }
    }
    Ok((peak, current_bytes))
}
