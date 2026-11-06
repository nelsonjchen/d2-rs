// SPDX-License-Identifier: MPL-2.0
//! d2raster/internal/scanline/rasterizer.go: exact-area non-zero scan
//! conversion of device-space edges. Cancellation checks are omitted (the
//! Rust port has no context); work accounting and every arithmetic step are
//! retained. ARM64 contraction sites follow the pinned assembly
//! (`cubeFlatEnough`, `coverage`); explicit `math.FMA` calls are always fused.

use std::fmt;

use crate::fma::{go_fmadd, go_fmadd32, go_fmsub};
use crate::pixels::{AlphaBuf, Rgba};
use d2_rs_scene::{Nrgba, go_max, go_min};

const CURVE_TOLERANCE: f64 = 0.375;
const MAX_CURVE_DEPTH: i32 = 18;

/// The scanline package's sentinel errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanError {
    EdgeLimit,
    CurveLimit,
    WorkLimit,
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ScanError::EdgeLimit => "scanline edge limit exceeded",
            ScanError::CurveLimit => "scanline curve subdivision limit exceeded",
            ScanError::WorkLimit => "scanline work limit exceeded",
        })
    }
}

/// Operation units shared across rasterizer calls for one frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorkBudget {
    remaining: i64,
}

impl WorkBudget {
    pub fn new(limit: i64) -> Self {
        Self {
            remaining: limit.max(0),
        }
    }

    pub fn remaining(&self) -> i64 {
        self.remaining
    }

    fn consume(&mut self, work: i64) -> Result<(), ScanError> {
        if work < 0 || work > self.remaining {
            return Err(ScanError::WorkLimit);
        }
        self.remaining -= work;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct P {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy, Debug)]
struct Edge {
    from: P,
    to: P,
}

#[derive(Clone, Copy, Debug, Default)]
struct ScanEdge {
    end: i64,
    next: usize,
    x: f64,
    y: f64,
    bottom: f64,
    slope: f64,
    inverse_slope: f64,
    winding: i8,
}

/// Reusable path and scanline storage. Not safe for concurrent use.
pub struct Rasterizer {
    width: i64,
    height: i64,
    edges: Vec<Edge>,
    scan_edges: Vec<ScanEdge>,
    active: Vec<usize>,
    row_heads: Vec<usize>,
    partial: Vec<f32>,
    difference: Vec<f32>,
    first: P,
    current: P,
    has_path: bool,
    origin_offset: P,
    touched_min: i64,
    touched_max: i64,
    touched: bool,
    count_only: bool,
    edge_count: i64,
    edge_limit: i64,
    path_err: Option<ScanError>,
    work_visible_edges: i64,
    work_active_rows: i64,
    work_horizontal_crossing: i64,
    work_first_row: i64,
    work_last_row: i64,
    work_touched_min: i64,
    work_touched_max: i64,
    work_touched: bool,
}

fn finite_point(p: P) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

fn checked_add_i64(left: i64, right: i64) -> Option<i64> {
    if left < 0 || right < 0 || right > i64::MAX - left {
        return None;
    }
    Some(left + right)
}

fn checked_mul_i64(left: i64, right: i64) -> Option<i64> {
    if left < 0 || right < 0 || left != 0 && right > i64::MAX / left {
        return None;
    }
    Some(left * right)
}

fn checked_mul_u64(left: u64, right: u64) -> Option<u64> {
    if left != 0 && right > u64::MAX / left {
        return None;
    }
    Some(left * right)
}

fn checked_add_u64(left: u64, right: u64) -> Option<u64> {
    if right > u64::MAX - left {
        return None;
    }
    Some(left + right)
}

/// unsafe.Sizeof(edge{}) + unsafe.Sizeof(scanEdge{}) + unsafe.Sizeof(int(0))
/// on 64-bit Go: 32 + 64 + 8.
const RETAINED_BYTES_PER_EDGE: u64 = 32 + 64 + 8;

/// RetainedBytes: backing storage retained for targets up to the given
/// maxima.
pub fn retained_bytes(max_width: i64, max_height: i64, max_edges: i64) -> Option<i64> {
    if max_width < 0 || max_height < 0 || max_edges < 0 {
        return None;
    }
    if max_width == 0 && max_height == 0 && max_edges == 0 {
        return Some(0);
    }
    let columns = checked_mul_u64(max_width as u64, 2 * 4)?;
    let columns = checked_add_u64(columns, 4)?;
    let rows = checked_mul_u64(max_height as u64, 8)?;
    let edges = checked_mul_u64(max_edges as u64, RETAINED_BYTES_PER_EDGE)?;
    let total = checked_add_u64(checked_add_u64(columns, rows)?, edges)?;
    if total > i64::MAX as u64 {
        return None;
    }
    Some(total as i64)
}

/// MaxEdgesForBytes.
pub fn max_edges_for_bytes(bytes: i64) -> i64 {
    if bytes <= 0 {
        return 0;
    }
    let count = bytes as u64 / RETAINED_BYTES_PER_EDGE;
    if count > i64::MAX as u64 {
        return i64::MAX;
    }
    count as i64
}

/// QuantizeCoverage / coverage: signed analytic area to 8-bit coverage.
#[inline]
pub fn quantize_coverage(area: f32) -> u8 {
    let mut area = area;
    if area < 0.0 {
        area = -area;
    }
    if area <= 0.0 {
        return 0;
    }
    if area >= 1.0 {
        return 255;
    }
    // ARM64 rasterizer.go:1320 FMADDS.
    go_fmadd32(area, 255.0, 0.5) as u8
}

impl Rasterizer {
    pub fn new(width: i64, height: i64) -> Self {
        let mut r = Self::empty(false, -1);
        r.reset(width, height);
        r
    }

    /// NewCounter: path flattening without edge storage.
    pub fn new_counter(width: i64, height: i64, edge_limit: i64) -> Self {
        let mut r = Self::empty(true, edge_limit.max(0));
        r.reset(width, height);
        r
    }

    fn empty(count_only: bool, edge_limit: i64) -> Self {
        Self {
            width: 0,
            height: 0,
            edges: Vec::new(),
            scan_edges: Vec::new(),
            active: Vec::new(),
            row_heads: Vec::new(),
            partial: Vec::new(),
            difference: Vec::new(),
            first: P::default(),
            current: P::default(),
            has_path: false,
            origin_offset: P::default(),
            touched_min: 0,
            touched_max: 0,
            touched: false,
            count_only,
            edge_count: 0,
            edge_limit,
            path_err: None,
            work_visible_edges: 0,
            work_active_rows: 0,
            work_horizontal_crossing: 0,
            work_first_row: 0,
            work_last_row: 0,
            work_touched_min: 0,
            work_touched_max: 0,
            work_touched: false,
        }
    }

    pub fn reset(&mut self, width: i64, height: i64) {
        if !self.count_only {
            self.clear_touched();
        }
        let width = width.max(0);
        let height = height.max(0);
        self.width = width;
        self.height = height;
        self.edges.clear();
        self.scan_edges.clear();
        self.active.clear();
        self.has_path = false;
        self.origin_offset = P::default();
        self.edge_count = 0;
        self.path_err = None;
        self.work_visible_edges = 0;
        self.work_active_rows = 0;
        self.work_horizontal_crossing = 0;
        self.work_first_row = height;
        self.work_last_row = 0;
        self.work_touched_min = width;
        self.work_touched_max = 0;
        self.work_touched = false;
        if !self.count_only {
            self.resize_rows(width);
        }
        self.reset_touched();
    }

    pub fn reserve_edges(&mut self, limit: i64) {
        let limit = limit.max(0);
        self.edge_limit = limit;
        if self.count_only {}
        // Capacity reservation is a Go allocation strategy; Rust vectors grow
        // on demand. The edge limit is what bounds path construction.
    }

    pub fn edge_count(&self) -> i64 {
        self.edge_count
    }

    pub fn err(&self) -> Option<ScanError> {
        self.path_err
    }

    fn resize_rows(&mut self, width: i64) {
        let width = width as usize;
        self.partial.resize(width, 0.0);
        self.difference.resize(width + 1, 0.0);
    }

    pub fn move_to32(&mut self, x: f32, y: f32) {
        self.move_to(P {
            x: x as f64,
            y: y as f64,
        });
    }

    pub fn set_origin_offset(&mut self, x: f64, y: f64) {
        self.origin_offset = P { x, y };
    }

    fn reference_point(&self, x: f64, y: f64) -> P {
        P {
            x: (x as f32) as f64 + self.origin_offset.x,
            y: (y as f32) as f64 + self.origin_offset.y,
        }
    }

    pub fn move_to64(&mut self, x: f64, y: f64) {
        let p = self.reference_point(x, y);
        self.move_to(p);
    }

    fn move_to(&mut self, p: P) {
        self.first = p;
        self.current = p;
        self.has_path = true;
    }

    pub fn line_to32(&mut self, x: f32, y: f32) {
        self.line_to(P {
            x: x as f64,
            y: y as f64,
        });
    }

    pub fn line_to64(&mut self, x: f64, y: f64) {
        let p = self.reference_point(x, y);
        self.line_to(p);
    }

    fn line_to(&mut self, next: P) {
        if !self.has_path {
            self.first = next;
            self.current = next;
            self.has_path = true;
            return;
        }
        self.add_edge(self.current, next);
        self.current = next;
    }

    pub fn cube_to64(&mut self, cx0: f64, cy0: f64, cx1: f64, cy1: f64, x: f64, y: f64) {
        let c0 = self.reference_point(cx0, cy0);
        let c1 = self.reference_point(cx1, cy1);
        let end = self.reference_point(x, y);
        self.cube_to(c0, c1, end);
    }

    fn cube_to(&mut self, c0: P, c1: P, end: P) {
        if !self.has_path {
            self.first = end;
            self.current = end;
            self.has_path = true;
            return;
        }
        self.flatten_cube(self.current, c0, c1, end, 0);
        self.current = end;
    }

    pub fn close_path(&mut self) {
        if !self.has_path {
            return;
        }
        self.add_edge(self.current, self.first);
        self.current = self.first;
        self.has_path = false;
    }

    fn add_edge(&mut self, from: P, to: P) {
        if self.path_err.is_some() {
            return;
        }
        if from == to || from.y == to.y {
            return;
        }
        if !finite_point(from) || !finite_point(to) {
            return;
        }
        if self.edge_limit >= 0 && self.edge_count >= self.edge_limit {
            self.path_err = Some(ScanError::EdgeLimit);
            return;
        }
        self.edge_count += 1;
        if !self.record_edge_work(from, to) {
            self.path_err = Some(ScanError::WorkLimit);
            return;
        }
        if self.count_only {
            return;
        }
        self.edges.push(Edge { from, to });
    }

    fn record_edge_work(&mut self, mut from: P, mut to: P) -> bool {
        if from.y > to.y {
            std::mem::swap(&mut from, &mut to);
        }
        if to.y <= 0.0 || from.y >= self.height as f64 {
            return true;
        }
        let start = go_max(from.y, 0.0) as i64;
        let end = go_min(to.y, self.height as f64).ceil() as i64;
        if start >= end {
            return true;
        }
        let row_span = end - start;
        let Some(visible_edges) = checked_add_i64(self.work_visible_edges, 1) else {
            return false;
        };
        let Some(active_rows) = checked_add_i64(self.work_active_rows, row_span) else {
            return false;
        };
        let width = self.width as f64;
        let clipped_from = go_min(go_max(from.x, 0.0), width);
        let clipped_to = go_min(go_max(to.x, 0.0), width);
        let mut delta_x = clipped_to - clipped_from;
        if delta_x < 0.0 {
            delta_x = -delta_x;
        }
        let horizontal = delta_x as i64 + 1;
        let Some(horizontal) = checked_add_i64(horizontal, row_span) else {
            return false;
        };
        let Some(horizontal) = checked_add_i64(self.work_horizontal_crossing, horizontal) else {
            return false;
        };
        let (min_x, max_x) = (go_min(from.x, to.x), go_max(from.x, to.x));
        let (edge_min, edge_max);
        if max_x <= 0.0 {
            edge_min = 0;
            edge_max = 0;
        } else if min_x >= width {
            edge_min = self.width;
            edge_max = self.width;
        } else {
            edge_min = go_max(min_x, 0.0) as i64;
            edge_max = if max_x >= width {
                self.width
            } else {
                max_x as i64 + 1
            };
        }
        if !self.work_touched {
            self.work_touched_min = edge_min;
            self.work_touched_max = edge_max;
            self.work_touched = true;
        } else {
            self.work_touched_min = self.work_touched_min.min(edge_min);
            self.work_touched_max = self.work_touched_max.max(edge_max);
        }
        self.work_visible_edges = visible_edges;
        self.work_active_rows = active_rows;
        self.work_horizontal_crossing = horizontal;
        self.work_first_row = self.work_first_row.min(start);
        self.work_last_row = self.work_last_row.max(end);
        true
    }

    fn flatten_cube(&mut self, p0: P, p1: P, p2: P, p3: P, depth: i32) {
        if self.path_err.is_some() {
            return;
        }
        let max_y = go_max(go_max(go_max(p0.y, p1.y), p2.y), p3.y);
        let min_y = go_min(go_min(go_min(p0.y, p1.y), p2.y), p3.y);
        if max_y <= 0.0 || min_y >= self.height as f64 {
            return;
        }
        if cube_flat_enough(p0, p1, p2, p3) {
            self.add_edge(p0, p3);
            return;
        }
        if depth >= MAX_CURVE_DEPTH {
            self.path_err = Some(ScanError::CurveLimit);
            return;
        }
        let p01 = midpoint(p0, p1);
        let p12 = midpoint(p1, p2);
        let p23 = midpoint(p2, p3);
        let p012 = midpoint(p01, p12);
        let p123 = midpoint(p12, p23);
        let middle = midpoint(p012, p123);
        self.flatten_cube(p0, p01, p012, middle, depth + 1);
        self.flatten_cube(middle, p123, p23, p3, depth + 1);
    }

    /// DrawRGBA paints the current paths over dst with a uniform
    /// non-premultiplied color. Go's sparse/opaque fast paths produce the same
    /// bytes as this general source-over loop.
    pub fn draw_rgba(
        &mut self,
        budget: &mut WorkBudget,
        dst: &mut Rgba<'_>,
        paint: Nrgba,
    ) -> Result<(), ScanError> {
        if let Some(err) = self.err() {
            return Err(err);
        }
        if self.width == 0 || self.height == 0 || self.edges.is_empty() || paint.a == 0 {
            return Ok(());
        }
        let (sr, sg, sb, sa) = nrgba_rgba(paint);
        let min_x0 = dst.rect.min.x;
        let min_y0 = dst.rect.min.y;
        self.rasterize(budget, |partial, difference, y, min_x, max_x| {
            let mut row = dst.pix_offset(min_x0 + min_x, min_y0 + y);
            let mut winding: f32 = 0.0;
            for x in min_x as usize..max_x as usize {
                winding += difference[x];
                let pixel_coverage = quantize_coverage(partial[x] + winding);
                if pixel_coverage != 0 {
                    let mut mask_alpha = pixel_coverage as u32;
                    mask_alpha |= mask_alpha << 8;
                    let inverse = (0xffffu32 - sa * mask_alpha / 0xffff).wrapping_mul(0x101);
                    let pixel = &mut dst.pix[row..row + 4];
                    pixel[0] = (((pixel[0] as u32)
                        .wrapping_mul(inverse)
                        .wrapping_add(sr * mask_alpha)
                        / 0xffff)
                        >> 8) as u8;
                    pixel[1] = (((pixel[1] as u32)
                        .wrapping_mul(inverse)
                        .wrapping_add(sg * mask_alpha)
                        / 0xffff)
                        >> 8) as u8;
                    pixel[2] = (((pixel[2] as u32)
                        .wrapping_mul(inverse)
                        .wrapping_add(sb * mask_alpha)
                        / 0xffff)
                        >> 8) as u8;
                    pixel[3] = (((pixel[3] as u32)
                        .wrapping_mul(inverse)
                        .wrapping_add(sa * mask_alpha)
                        / 0xffff)
                        >> 8) as u8;
                }
                row += 4;
            }
            Ok(())
        })
    }

    /// WriteAlpha writes opaque path coverage into a freshly zeroed dst.
    pub fn write_alpha(
        &mut self,
        budget: &mut WorkBudget,
        dst: &mut AlphaBuf,
    ) -> Result<(), ScanError> {
        if let Some(err) = self.err() {
            return Err(err);
        }
        if self.width == 0 || self.height == 0 || self.edges.is_empty() {
            return Ok(());
        }
        let min_x0 = dst.rect.min.x;
        let min_y0 = dst.rect.min.y;
        self.rasterize(budget, |partial, difference, y, min_x, max_x| {
            let mut row = dst.pix_offset(min_x0 + min_x, min_y0 + y);
            let mut winding: f32 = 0.0;
            for x in min_x as usize..max_x as usize {
                winding += difference[x];
                dst.pix[row] = quantize_coverage(partial[x] + winding);
                row += 1;
            }
            Ok(())
        })
    }

    /// WalkCoverage visits each row segment that can contain coverage with
    /// views of partial and difference aligned at minX.
    pub fn walk_coverage<E>(
        &mut self,
        budget: &mut WorkBudget,
        mut write_row: impl FnMut(i64, i64, &[f32], &[f32]) -> Result<(), E>,
    ) -> Result<(), WalkError<E>> {
        if let Some(err) = self.err() {
            return Err(WalkError::Scan(err));
        }
        if self.width == 0 || self.height == 0 || self.edges.is_empty() {
            return Ok(());
        }
        let mut callback_error = None;
        let result = self.rasterize(budget, |partial, difference, y, min_x, max_x| {
            match write_row(
                y,
                min_x,
                &partial[min_x as usize..max_x as usize],
                &difference[min_x as usize..max_x as usize],
            ) {
                Ok(()) => Ok(()),
                Err(err) => {
                    callback_error = Some(err);
                    // Any sentinel stops the scan; the callback error wins.
                    Err(ScanError::WorkLimit)
                }
            }
        });
        if let Some(err) = callback_error {
            return Err(WalkError::Callback(err));
        }
        result.map_err(WalkError::Scan)
    }

    /// WorkBound from aggregate edge metrics.
    pub fn work_bound(&self) -> Option<i64> {
        if self.edge_count == 0 {
            return Some(0);
        }
        let rows = if self.work_first_row < self.work_last_row {
            self.work_last_row - self.work_first_row
        } else {
            0
        };
        let touched_width = if self.work_touched_min < self.work_touched_max {
            self.work_touched_max - self.work_touched_min
        } else {
            0
        };
        let painted = checked_mul_i64(checked_mul_i64(touched_width, rows)?, 3)?;
        let visible_work = checked_mul_i64(self.work_visible_edges, 2)?;
        let active_work = checked_mul_i64(self.work_active_rows, 2)?;
        let row_work = checked_mul_i64(rows, 2)?;
        let terms = [
            self.height,
            self.edge_count,
            visible_work,
            active_work,
            self.work_horizontal_crossing,
            row_work,
            painted,
        ];
        let mut total = 0i64;
        for term in terms {
            if term < 0 {
                return None;
            }
            total = checked_add_i64(total, term)?;
        }
        Some(total)
    }

    fn rasterize(
        &mut self,
        budget: &mut WorkBudget,
        mut write_row: impl FnMut(&[f32], &[f32], i64, i64, i64) -> Result<(), ScanError>,
    ) -> Result<(), ScanError> {
        let work = self.work_bound().ok_or(ScanError::WorkLimit)?;
        budget.consume(work)?;
        let (first_row, last_row) = self.prepare_scan_edges();
        if first_row >= last_row {
            return Ok(());
        }
        self.active.clear();
        let mut result = Ok(());
        let mut next_head_row = first_row;
        for y in first_row..last_row {
            self.remove_expired(y);
            let mut head = self.row_heads[y as usize];
            self.row_heads[y as usize] = 0;
            next_head_row = y + 1;
            while head != 0 {
                let index = head - 1;
                self.active.push(index);
                head = self.scan_edges[index].next;
            }
            for i in 0..self.active.len() {
                let index = self.active[i];
                self.accumulate_edge_row(index, y);
            }
            if self.touched_min < self.touched_max {
                if let Err(err) = write_row(
                    &self.partial,
                    &self.difference,
                    y,
                    self.touched_min,
                    self.touched_max,
                ) {
                    result = Err(err);
                    self.clear_touched();
                    break;
                }
            }
            self.clear_touched();
        }
        self.clear_row_heads(next_head_row, last_row);
        result
    }

    fn prepare_scan_edges(&mut self) -> (i64, i64) {
        self.scan_edges.clear();
        self.row_heads.resize(self.height as usize, 0);
        let mut first_row = self.height;
        let mut last_row = 0;
        let height = self.height as f64;
        for raw_index in 0..self.edges.len() {
            let raw = self.edges[raw_index];
            let (mut from, mut to) = (raw.from, raw.to);
            let mut winding = 1i8;
            if from.y > to.y {
                std::mem::swap(&mut from, &mut to);
                winding = -1;
            }
            if to.y <= 0.0 || from.y >= height {
                continue;
            }
            let start = go_max(from.y, 0.0).floor() as i64;
            let end = go_min(to.y, height).ceil() as i64;
            if start >= end {
                continue;
            }
            let slope = (to.x - from.x) / (to.y - from.y);
            let top = go_max(from.y, start as f64);
            let x = (top - from.y).mul_add(slope, from.x);
            let inverse_slope = if slope != 0.0 { 1.0 / slope } else { 0.0 };
            let index = self.scan_edges.len();
            self.scan_edges.push(ScanEdge {
                end,
                next: self.row_heads[start as usize],
                x,
                y: top,
                bottom: to.y,
                slope,
                inverse_slope,
                winding,
            });
            self.row_heads[start as usize] = index + 1;
            if start < first_row {
                first_row = start;
            }
            if end > last_row {
                last_row = end;
            }
        }
        (first_row, last_row)
    }

    fn clear_row_heads(&mut self, first_row: i64, last_row: i64) {
        if first_row >= last_row {
            return;
        }
        for head in &mut self.row_heads[first_row as usize..last_row as usize] {
            *head = 0;
        }
    }

    fn remove_expired(&mut self, row: i64) {
        let mut index = 0;
        while index < self.active.len() {
            if self.scan_edges[self.active[index]].end > row {
                index += 1;
                continue;
            }
            let last = self.active.len() - 1;
            self.active[index] = self.active[last];
            self.active.truncate(last);
        }
    }

    fn accumulate_edge_row(&mut self, edge_index: usize, row: i64) {
        let width = self.width as f64;
        let active = self.scan_edges[edge_index];
        let bottom = go_min(active.bottom, (row + 1) as f64);
        let height = bottom - active.y;
        if height <= 0.0 {
            return;
        }
        let x_top = active.x;
        let x_bottom = height.mul_add(active.slope, x_top);
        let top = active.y;
        self.scan_edges[edge_index].x = x_bottom;
        self.scan_edges[edge_index].y = bottom;
        let sign = active.winding as f64;
        let delta_x = x_bottom - x_top;
        if delta_x == 0.0 {
            if x_top < 0.0 {
                self.add_difference_event(0, sign * height);
            } else if x_top >= width {
                self.add_difference_event(self.width, sign * height);
            } else {
                let pixel = x_top.floor() as i64;
                self.accumulate_visible_edge_portion(pixel, sign, x_top, x_bottom, height);
                self.touch(pixel, pixel + 1);
            }
            return;
        }

        let dy_per_x = active.inverse_slope;
        let (mut current_x, mut current_y) = (x_top, top);
        if delta_x > 0.0 {
            if current_x < 0.0 {
                if x_bottom <= 0.0 {
                    self.add_difference_event(0, sign * height);
                    return;
                }
                let cross_y = (-current_x).mul_add(dy_per_x, current_y);
                self.add_difference_event(0, sign * (cross_y - current_y));
                current_x = 0.0;
                current_y = cross_y;
            }
            if current_x >= width {
                self.add_difference_event(self.width, sign * (bottom - current_y));
                return;
            }
            let visible_end_x = go_min(x_bottom, width);
            let visible_end_y = (visible_end_x - current_x).mul_add(dy_per_x, current_y);
            self.touch(
                current_x.floor() as i64,
                self.width.min(visible_end_x.floor() as i64 + 1),
            );
            let mut boundary = current_x.floor() + 1.0;
            if boundary < visible_end_x {
                let next_y = (boundary - current_x).mul_add(dy_per_x, current_y);
                self.accumulate_visible_edge_portion(
                    boundary as i64 - 1,
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
                    let pixel = boundary as i64 - 1;
                    self.partial[pixel as usize] += partial;
                    self.difference[pixel as usize + 1] += difference;
                    current_x = boundary;
                    current_y += dy_per_x;
                    boundary += 1.0;
                }
            }
            self.accumulate_visible_edge_portion(
                ((current_x + visible_end_x) * 0.5).floor() as i64,
                sign,
                current_x,
                visible_end_x,
                visible_end_y - current_y,
            );
            if visible_end_y < bottom {
                self.add_difference_event(self.width, sign * (bottom - visible_end_y));
            }
            return;
        }

        if current_x > width {
            if x_bottom >= width {
                self.add_difference_event(self.width, sign * height);
                return;
            }
            let cross_y = (width - current_x).mul_add(dy_per_x, current_y);
            self.add_difference_event(self.width, sign * (cross_y - current_y));
            current_x = width;
            current_y = cross_y;
        }
        if current_x <= 0.0 {
            self.add_difference_event(0, sign * (bottom - current_y));
            return;
        }
        let visible_end_x = go_max(x_bottom, 0.0);
        let visible_end_y = (visible_end_x - current_x).mul_add(dy_per_x, current_y);
        self.touch(
            visible_end_x.floor() as i64,
            self.width.min(current_x.floor() as i64 + 1),
        );
        let mut boundary = current_x.ceil() - 1.0;
        if boundary > visible_end_x {
            let next_y = (boundary - current_x).mul_add(dy_per_x, current_y);
            self.accumulate_visible_edge_portion(
                boundary as i64,
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
                let pixel = boundary as i64;
                self.partial[pixel as usize] += partial;
                self.difference[pixel as usize + 1] += difference;
                current_x = boundary;
                current_y += full_height;
                boundary -= 1.0;
            }
        }
        self.accumulate_visible_edge_portion(
            ((current_x + visible_end_x) * 0.5).floor() as i64,
            sign,
            current_x,
            visible_end_x,
            visible_end_y - current_y,
        );
        if visible_end_y < bottom {
            self.add_difference_event(0, sign * (bottom - visible_end_y));
        }
    }

    fn accumulate_visible_edge_portion(
        &mut self,
        pixel: i64,
        sign: f64,
        from_x: f64,
        to_x: f64,
        height: f64,
    ) {
        if height <= 0.0 {
            return;
        }
        // ARM64 rasterizer.go:1298 FMSUBD; the product by 0.5 is exact.
        let middle_x = (from_x + to_x) * 0.5;
        self.partial[pixel as usize] += (sign * height * ((pixel + 1) as f64 - middle_x)) as f32;
        self.difference[pixel as usize + 1] += (sign * height) as f32;
    }

    fn add_difference_event(&mut self, x: i64, amount: f64) {
        if amount == 0.0 {
            return;
        }
        self.difference[x as usize] += amount as f32;
        self.touch_event(x);
    }

    fn reset_touched(&mut self) {
        self.touched_min = self.width;
        self.touched_max = 0;
        self.touched = false;
    }

    fn touch(&mut self, min_x: i64, max_x: i64) {
        self.touched = true;
        if min_x < self.touched_min {
            self.touched_min = min_x;
        }
        if max_x > self.touched_max {
            self.touched_max = max_x;
        }
    }

    fn touch_event(&mut self, x: i64) {
        self.touched = true;
        if x < self.touched_min {
            self.touched_min = x;
        }
        if x > self.touched_max {
            self.touched_max = x;
        }
    }

    fn clear_touched(&mut self) {
        if !self.touched {
            return;
        }
        let partial_max = self.touched_max.min(self.width);
        if self.touched_min < partial_max {
            for value in &mut self.partial[self.touched_min as usize..partial_max as usize] {
                *value = 0.0;
            }
        }
        let difference_end = (self.touched_max.min(self.width) + 1) as usize;
        if (self.touched_min as usize) < difference_end {
            for value in &mut self.difference[self.touched_min as usize..difference_end] {
                *value = 0.0;
            }
        }
        self.reset_touched();
    }
}

/// An error from WalkCoverage: either scan conversion or the row callback.
#[derive(Debug)]
pub enum WalkError<E> {
    Scan(ScanError),
    Callback(E),
}

/// color.NRGBA.RGBA(): 16-bit premultiplied channels.
pub fn nrgba_rgba(c: Nrgba) -> (u32, u32, u32, u32) {
    let a = c.a as u32;
    let channel = |v: u8| {
        let mut x = v as u32;
        x |= x << 8;
        x * a / 0xff
    };
    (channel(c.r), channel(c.g), channel(c.b), a | a << 8)
}

fn midpoint(a: P, b: P) -> P {
    // ARM64 rasterizer.go:454: the products by 0.5 are exact.
    P {
        x: a.x + (b.x - a.x) * 0.5,
        y: a.y + (b.y - a.y) * 0.5,
    }
}

fn cube_flat_enough(p0: P, p1: P, p2: P, p3: P) -> bool {
    let dx = p3.x - p0.x;
    let dy = p3.y - p0.y;
    // ARM64 rasterizer.go:460: dx*dx is the fused product.
    let length_squared = go_fmadd(dx, dx, dy * dy);
    if length_squared == 0.0 {
        let (d1x, d1y) = (p1.x - p0.x, p1.y - p0.y);
        let (d2x, d2y) = (p2.x - p0.x, p2.y - p0.y);
        // ARM64 rasterizer.go:464.
        return go_max(go_fmadd(d1x, d1x, d1y * d1y), go_fmadd(d2x, d2x, d2y * d2y))
            <= CURVE_TOLERANCE * CURVE_TOLERANCE;
    }
    let limit = CURVE_TOLERANCE * CURVE_TOLERANCE * length_squared;
    let sqrt_length = length_squared.sqrt();
    let mut previous_projection = 0.0;
    for control in [p1, p2] {
        let control_x = control.x - p0.x;
        let control_y = control.y - p0.y;
        // ARM64 rasterizer.go:472–473.
        let projection = go_fmadd(control_x, dx, control_y * dy);
        let cross = go_fmsub(dy, control_x, dx * control_y);
        let mut excess = 0.0;
        if projection < 0.0 {
            excess = -projection;
        } else if projection > length_squared {
            excess = projection - length_squared;
        }
        // ARM64 rasterizer.go:480: FMADDD and the fused tolerance FMSUBD.
        if go_fmadd(cross, cross, excess * excess) > limit
            || projection < go_fmsub(CURVE_TOLERANCE, sqrt_length, previous_projection)
        {
            return false;
        }
        previous_projection = projection;
    }
    true
}
