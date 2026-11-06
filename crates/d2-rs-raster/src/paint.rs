// SPDX-License-Identifier: MPL-2.0
//! d2raster/gradient.go, pattern.go and color_space.go: prepared paints,
//! gradient and pattern sampling, streamed paint coverage and the integer
//! source-over compositor. Contraction sites were read from each enclosing
//! function's ARM64 assembly; they differ between colorAt and the
//! specialized row loops, so each loop keeps its own expression.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use d2_rs_scene::{
    GradientStop, LinearGradient, Matrix, Nrgba, Paint, PaintUnits, PatternPaint, Point,
    RadialGradient, SpreadMethod, go_max, go_min,
};

use crate::fma::{
    IDENTITY, finite, finite_point, go_fmadd, go_fmsub, go_fnmsub, gomath, mmul, mpoint, scale,
    translate,
};
use crate::pixels::{AlphaBuf, Rect, Rgba, RgbaBuf};
use crate::raster::{PreparedNode, Scratch, render_node};
use crate::scanline::{Rasterizer, WalkError, WorkBudget, quantize_coverage};
use crate::vector_validation::finite_affine_inverse;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintKind {
    Solid,
    Linear,
    LinearX,
    LinearY,
    LinearXOnly,
    LinearYOnly,
    Radial,
    Pattern,
}

#[derive(Clone, Debug)]
pub struct PreparedPaint {
    pub kind: PaintKind,
    pub solid: Nrgba,
    pub gradient: Rc<PreparedGradient>,
    pub pattern: Option<Rc<PreparedPattern>>,
}

impl PreparedPaint {
    pub fn solid(color: Nrgba) -> Self {
        Self {
            kind: PaintKind::Solid,
            solid: color,
            gradient: empty_gradient(),
            pattern: None,
        }
    }

    fn gradient(kind: PaintKind, gradient: PreparedGradient) -> Self {
        Self {
            kind,
            solid: Nrgba::default(),
            gradient: Rc::new(gradient),
            pattern: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PreparedGradient {
    pub device_to_gradient: Matrix,
    pub stops: Vec<GradientStop>,
    pub spread: SpreadMethod,
    pub colrv1_interpolation: bool,
    pub linear_start: Point,
    pub linear_delta: Point,
    pub linear_denominator: f64,
    pub radial_focal: Point,
    pub radial_delta: Point,
    pub radial_focal_radius: f64,
    pub radial_delta_radius: f64,
    pub radial_a: f64,
    pub radial_tangent: bool,
    pub radial_concentric: bool,
    pub radial_average: Nrgba,
}

/// gradient.go leaves the inline gradient at its zero value for solid and
/// pattern paints. Share that unused immutable value rather than allocating
/// one for every prepared paint, including repeated connection-mask nodes.
pub(crate) fn empty_gradient() -> Rc<PreparedGradient> {
    thread_local! {
        static EMPTY: Rc<PreparedGradient> = Rc::default();
    }
    EMPTY.with(Rc::clone)
}

/// The patternPaintPreparer callback.
pub type PatternPreparer<'a> =
    &'a mut dyn FnMut(&PatternPaint, d2_rs_scene::Box, &Matrix) -> Result<PreparedPaint, String>;

pub fn prepare_animated_paint_with_pattern(
    paint: Option<&Paint>,
    animated_color: Option<Nrgba>,
    object_bounds: d2_rs_scene::Box,
    object_to_device: &Matrix,
    prepare_pattern: Option<PatternPreparer<'_>>,
) -> Result<Option<PreparedPaint>, String> {
    match paint {
        None => {
            if animated_color.is_some() {
                return Err("color animation targets missing paint".into());
            }
            Ok(None)
        }
        Some(Paint::SolidPaint { color }) => {
            Ok(Some(PreparedPaint::solid(animated_color.unwrap_or(*color))))
        }
        Some(Paint::LinearGradient(gradient)) => {
            if animated_color.is_some() {
                return Err("color animation targets non-solid paint".into());
            }
            prepare_linear_gradient(gradient, object_bounds, object_to_device).map(Some)
        }
        Some(Paint::RadialGradient(gradient)) => {
            if animated_color.is_some() {
                return Err("color animation targets non-solid paint".into());
            }
            prepare_radial_gradient(gradient, object_bounds, object_to_device).map(Some)
        }
        Some(Paint::PatternPaint(pattern)) => {
            if animated_color.is_some() {
                return Err("color animation targets pattern paint".into());
            }
            let Some(prepare) = prepare_pattern else {
                return Err("unsupported pattern paint without render preflight".into());
            };
            prepare(pattern, object_bounds, object_to_device).map(Some)
        }
    }
}

pub(crate) fn prepare_linear_gradient(
    gradient: &LinearGradient,
    object_bounds: d2_rs_scene::Box,
    object_to_device: &Matrix,
) -> Result<PreparedPaint, String> {
    if !finite_point(gradient.start) || !finite_point(gradient.end) {
        return Err("linear gradient has non-finite geometry".into());
    }
    let device_to_gradient = prepare_gradient_transform(
        gradient.units,
        &gradient.transform,
        object_bounds,
        object_to_device,
    )
    .map_err(|e| format!("linear gradient: {e}"))?;
    let stops =
        normalize_gradient_stops(&gradient.stops).map_err(|e| format!("linear gradient: {e}"))?;
    if gradient.spread.0 > SpreadMethod::REPEAT.0 {
        return Err(format!(
            "linear gradient has invalid spread method {}",
            gradient.spread.0
        ));
    }
    let delta = Point::new(
        gradient.end.x - gradient.start.x,
        gradient.end.y - gradient.start.y,
    );
    // ARM64 gradient.go:137: delta.Y*delta.Y is the fused product.
    let denominator = go_fmadd(delta.y, delta.y, delta.x * delta.x);
    if !finite(denominator) {
        return Err("linear gradient vector is outside the finite numeric domain".into());
    }
    if denominator == 0.0 || stops.len() == 1 {
        return Ok(PreparedPaint::solid(stops[stops.len() - 1].color));
    }
    let mut kind = PaintKind::Linear;
    let m = device_to_gradient;
    if delta.y == 0.0 && affine_coordinate_finite_for_raster(m.b, m.d, m.f, gradient.start.y) {
        kind = if m.c == 0.0 {
            PaintKind::LinearXOnly
        } else {
            PaintKind::LinearX
        };
    } else if delta.x == 0.0 && affine_coordinate_finite_for_raster(m.a, m.c, m.e, gradient.start.x)
    {
        kind = if m.b == 0.0 {
            PaintKind::LinearYOnly
        } else {
            PaintKind::LinearY
        };
    }
    Ok(PreparedPaint::gradient(
        kind,
        PreparedGradient {
            device_to_gradient,
            stops,
            spread: gradient.spread,
            linear_start: gradient.start,
            linear_delta: delta,
            linear_denominator: denominator,
            ..Default::default()
        },
    ))
}

fn affine_coordinate_finite_for_raster(a: f64, b: f64, offset: f64, origin: f64) -> bool {
    let maximum_coordinate = i64::MAX as f64;
    let mut remaining = f64::MAX / 4.0;
    for (factor, term_scale) in [
        (a, maximum_coordinate),
        (b, maximum_coordinate),
        (offset, 1.0),
        (origin, 1.0),
    ] {
        let magnitude = factor.abs();
        if magnitude > remaining / term_scale {
            return false;
        }
        // ARM64 gradient.go:195.
        remaining = go_fmsub(magnitude, term_scale, remaining);
    }
    true
}

pub(crate) fn prepare_radial_gradient(
    gradient: &RadialGradient,
    object_bounds: d2_rs_scene::Box,
    object_to_device: &Matrix,
) -> Result<PreparedPaint, String> {
    if !finite_point(gradient.center)
        || !finite_point(gradient.focal)
        || !finite(gradient.radius)
        || !finite(gradient.focal_radius)
    {
        return Err("radial gradient has non-finite geometry".into());
    }
    if gradient.radius < 0.0 || gradient.focal_radius < 0.0 {
        return Err("radial gradient has negative radius".into());
    }
    let device_to_gradient = prepare_gradient_transform(
        gradient.units,
        &gradient.transform,
        object_bounds,
        object_to_device,
    )
    .map_err(|e| format!("radial gradient: {e}"))?;
    let stops =
        normalize_gradient_stops(&gradient.stops).map_err(|e| format!("radial gradient: {e}"))?;
    if gradient.spread.0 > SpreadMethod::REPEAT.0 {
        return Err(format!(
            "radial gradient has invalid spread method {}",
            gradient.spread.0
        ));
    }
    if stops.len() == 1 {
        return Ok(PreparedPaint::solid(stops[0].color));
    }
    let delta = Point::new(
        gradient.center.x - gradient.focal.x,
        gradient.center.y - gradient.focal.y,
    );
    let delta_radius = gradient.radius - gradient.focal_radius;
    // ARM64 gradient.go:223 and 231: delta.X*delta.X fused onto delta.Y*delta.Y.
    let squared = go_fmadd(delta.x, delta.x, delta.y * delta.y);
    let a = go_fmsub(delta_radius, delta_radius, squared);
    if !finite(a) || !finite(delta_radius) {
        return Err("radial gradient cone is outside the finite numeric domain".into());
    }
    if delta.x == 0.0 && delta.y == 0.0 && delta_radius == 0.0 {
        return Ok(PreparedPaint::solid(Nrgba::default()));
    }
    let a_scale = go_fmadd(delta_radius, delta_radius, squared) + 1.0;
    let radial_average = average_gradient_color(&stops);
    Ok(PreparedPaint::gradient(
        PaintKind::Radial,
        PreparedGradient {
            device_to_gradient,
            stops,
            spread: gradient.spread,
            radial_focal: gradient.focal,
            radial_delta: delta,
            radial_focal_radius: gradient.focal_radius,
            radial_delta_radius: delta_radius,
            radial_a: a,
            radial_tangent: a.abs() <= 1e-14 * a_scale,
            radial_concentric: delta.x == 0.0 && delta.y == 0.0 && gradient.focal_radius == 0.0,
            radial_average,
            ..Default::default()
        },
    ))
}

/// The units-to-object mapping shared by gradient and pattern transforms.
fn units_to_object(units: PaintUnits, object_bounds: d2_rs_scene::Box) -> Result<Matrix, String> {
    let mut units_to_object = IDENTITY;
    if units == PaintUnits::OBJECT_BOUNDING_BOX {
        crate::raster::validate_box(object_bounds)
            .map_err(|e| format!("invalid object bounding box: {e}"))?;
        if object_bounds.width == 0.0 || object_bounds.height == 0.0 {
            return Err("object bounding box has zero width or height".into());
        }
        units_to_object = mmul(
            &translate(object_bounds.x, object_bounds.y),
            &scale(object_bounds.width, object_bounds.height),
        );
    }
    Ok(units_to_object)
}

fn prepare_gradient_transform(
    units: PaintUnits,
    gradient_transform: &Matrix,
    object_bounds: d2_rs_scene::Box,
    object_to_device: &Matrix,
) -> Result<Matrix, String> {
    if units.0 > PaintUnits::USER_SPACE_ON_USE.0 {
        return Err(format!("invalid paint units {}", units.0));
    }
    if !gradient_transform.is_finite() {
        return Err("non-finite gradient transform".into());
    }
    if !object_to_device.is_finite() {
        return Err("non-finite object transform".into());
    }
    let units_to_object = units_to_object(units, object_bounds)?;
    let gradient_to_device = mmul(
        &mmul(object_to_device, &units_to_object),
        gradient_transform,
    );
    if !gradient_to_device.is_finite() {
        return Err("composed gradient transform is non-finite".into());
    }
    match finite_affine_inverse(&gradient_to_device) {
        Ok(Some(inverse)) => Ok(inverse),
        _ => Err("singular gradient transform".into()),
    }
}

fn normalize_gradient_stops(stops: &[GradientStop]) -> Result<Vec<GradientStop>, String> {
    if stops.is_empty() {
        return Err("gradient has no stops".into());
    }
    for (index, stop) in stops.iter().enumerate() {
        if !finite(stop.offset) {
            return Err(format!("gradient stop {index} has non-finite offset"));
        }
    }
    let mut normalized = Vec::with_capacity(stops.len());
    let mut previous = 0.0;
    for (index, stop) in stops.iter().enumerate() {
        let mut stop = *stop;
        stop.offset = go_max(0.0, go_min(1.0, stop.offset));
        if index != 0 && stop.offset < previous {
            stop.offset = previous;
        }
        previous = stop.offset;
        normalized.push(stop);
    }
    Ok(normalized)
}

impl PreparedPaint {
    pub fn color_at(&self, x: f64, y: f64) -> Option<Nrgba> {
        match self.kind {
            PaintKind::Solid => return (self.solid.a != 0).then_some(self.solid),
            PaintKind::Pattern => return self.pattern.as_ref().and_then(|p| p.color_at(x, y)),
            _ => {}
        }
        let gradient = &*self.gradient;
        let m = &gradient.device_to_gradient;
        let parameter = match self.kind {
            PaintKind::Linear => {
                let point = mpoint(m, Point::new(x, y));
                // ARM64 gradient.go:342: the X term is fused in colorAt.
                go_fmadd(
                    point.x - gradient.linear_start.x,
                    gradient.linear_delta.x,
                    (point.y - gradient.linear_start.y) * gradient.linear_delta.y,
                ) / gradient.linear_denominator
            }
            PaintKind::LinearX => {
                // ARM64 gradient.go:345.
                let point_x = go_fmadd(m.a, x, m.c * y) + m.e;
                (point_x - gradient.linear_start.x) * gradient.linear_delta.x
                    / gradient.linear_denominator
            }
            PaintKind::LinearY => {
                // ARM64 gradient.go:349: D*y is the fused product.
                let point_y = go_fmadd(m.d, y, m.b * x) + m.f;
                (point_y - gradient.linear_start.y) * gradient.linear_delta.y
                    / gradient.linear_denominator
            }
            PaintKind::LinearXOnly => {
                let point_x = go_fmadd(m.a, x, m.e);
                (point_x - gradient.linear_start.x) * gradient.linear_delta.x
                    / gradient.linear_denominator
            }
            PaintKind::LinearYOnly => {
                let point_y = go_fmadd(m.d, y, m.f);
                (point_y - gradient.linear_start.y) * gradient.linear_delta.y
                    / gradient.linear_denominator
            }
            PaintKind::Radial => {
                let point = mpoint(m, Point::new(x, y));
                match gradient.radial_parameter(point) {
                    Some(t) => t,
                    None => {
                        if gradient.spread == SpreadMethod::REPEAT && gradient.radial_tangent {
                            return (gradient.radial_average.a != 0)
                                .then_some(gradient.radial_average);
                        }
                        return None;
                    }
                }
            }
            _ => return None,
        };
        if !finite(parameter) {
            return None;
        }
        Some(gradient.sample(spread_parameter(parameter, gradient.spread)))
    }
}

impl PreparedGradient {
    fn sample(&self, parameter: f64) -> Nrgba {
        if self.colrv1_interpolation {
            interpolate_colrv1_gradient_stops(&self.stops, parameter)
        } else {
            interpolate_gradient_stops(&self.stops, parameter)
        }
    }

    fn sample_spread(&self, parameter: f64) -> Nrgba {
        self.sample(spread_parameter(parameter, self.spread))
    }

    pub fn radial_parameter(&self, point: Point) -> Option<f64> {
        let qx = point.x - self.radial_focal.x;
        let qy = point.y - self.radial_focal.y;
        // ARM64 gradient.go:386: qx*qx fused onto qy*qy, then FMSUBD.
        let c = go_fmsub(
            self.radial_focal_radius,
            self.radial_focal_radius,
            go_fmadd(qx, qx, qy * qy),
        );
        let a = self.radial_a;
        if self.radial_concentric {
            let root = go_fmsub(a, c, 0.0).sqrt();
            for t in [root / a, -root / a] {
                if self.radial_solution_valid(t) {
                    return Some(t);
                }
            }
            return None;
        }
        // ARM64 gradient.go:397: qx term fused onto the qy product, then the
        // radius product fused.
        let b = go_fmadd(
            self.radial_focal_radius,
            self.radial_delta_radius,
            go_fmadd(qx, self.radial_delta.x, qy * self.radial_delta.y),
        );
        if self.radial_tangent {
            let b_scale = (qx * self.radial_delta.x).abs()
                + (qy * self.radial_delta.y).abs()
                + (self.radial_focal_radius * self.radial_delta_radius).abs()
                + 1.0;
            if b.abs() <= 1e-14 * b_scale {
                return None;
            }
            let t = 0.5 * c / b;
            return self.radial_solution_valid(t).then_some(t);
        }
        let mut discriminant = go_fmsub(a, c, b * b);
        if discriminant < 0.0 {
            let discriminant_scale = (b * b).abs() + (a * c).abs() + 1.0;
            if discriminant >= -1e-14 * discriminant_scale {
                discriminant = 0.0;
            } else {
                return None;
            }
        }
        let root = discriminant.sqrt();
        [(b + root) / a, (b - root) / a]
            .into_iter()
            .find(|&t| self.radial_solution_valid(t))
    }

    fn radial_solution_valid(&self, t: f64) -> bool {
        if !finite(t) {
            return false;
        }
        let radius = go_fmadd(t, self.radial_delta_radius, self.radial_focal_radius);
        finite(radius) && radius >= -1e-12
    }
}

pub fn spread_parameter(mut value: f64, spread: SpreadMethod) -> f64 {
    match spread {
        SpreadMethod::PAD => go_max(0.0, go_min(1.0, value)),
        SpreadMethod::REFLECT => {
            if value > -2.0 && value < 2.0 {
                if value < 0.0 {
                    value += 2.0;
                }
                if value > 1.0 {
                    value = 2.0 - value;
                }
                return value;
            }
            value %= 2.0;
            if value < 0.0 {
                value += 2.0;
            }
            if value > 1.0 {
                value = 2.0 - value;
            }
            value
        }
        SpreadMethod::REPEAT => {
            if value == 0.0 {
                return 0.0;
            }
            if value > 0.0 && value < 1.0 {
                return value;
            }
            value - value.floor()
        }
        _ => value,
    }
}

fn stop_pair(stops: &[GradientStop], value: f64) -> (GradientStop, GradientStop) {
    match stops.len() {
        2 => (stops[0], stops[1]),
        3 => {
            if stops[1].offset <= value {
                (stops[1], stops[2])
            } else {
                (stops[0], stops[1])
            }
        }
        _ => {
            let (mut low, mut high) = (0usize, stops.len());
            while low < high {
                let middle = low + (high - low) / 2;
                if stops[middle].offset <= value {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            (stops[low - 1], stops[low])
        }
    }
}

pub fn interpolate_gradient_stops(stops: &[GradientStop], value: f64) -> Nrgba {
    if stops.len() == 1 || value < stops[0].offset {
        return stops[0].color;
    }
    if value >= stops[stops.len() - 1].offset {
        return stops[stops.len() - 1].color;
    }
    let (left, right) = stop_pair(stops, value);
    let amount = (value - left.offset) / (right.offset - left.offset);
    lerp_nrgba(left.color, right.color, amount)
}

fn lerp_nrgba(left: Nrgba, right: Nrgba, amount: f64) -> Nrgba {
    // ARM64 gradient.go:515.
    let lerp = |a: u8, b: u8| go_fmadd(b as f64 - a as f64, amount, a as f64).round() as u8;
    Nrgba::new(
        lerp(left.r, right.r),
        lerp(left.g, right.g),
        lerp(left.b, right.b),
        lerp(left.a, right.a),
    )
}

fn interpolate_colrv1_gradient_stops(stops: &[GradientStop], value: f64) -> Nrgba {
    if stops.len() == 1 || value < stops[0].offset {
        return stops[0].color;
    }
    if value >= stops[stops.len() - 1].offset {
        return stops[stops.len() - 1].color;
    }
    let (left, right) = stop_pair(stops, value);
    let amount = (value - left.offset) / (right.offset - left.offset);
    lerp_colrv1_color(left.color, right.color, amount)
}

fn lerp_colrv1_color(left: Nrgba, right: Nrgba, amount: f64) -> Nrgba {
    let left_alpha = left.a as f64 / 255.0;
    let right_alpha = right.a as f64 / 255.0;
    let alpha = go_fmadd(right_alpha - left_alpha, amount, left_alpha);
    let mut result = Nrgba::new(0, 0, 0, rounded_byte(alpha * 255.0));
    if alpha <= 0.0 {
        return result;
    }
    let table = linear_srgb_byte();
    let interpolate = |left_channel: u8, right_channel: u8| {
        let left_linear = table[left_channel as usize] * left_alpha;
        let right_linear = table[right_channel as usize] * right_alpha;
        // ARM64 gradient.go:572: FMSUBD then FMADDD onto the rounded left.
        let difference = go_fmsub(table[left_channel as usize], left_alpha, right_linear);
        let premultiplied = go_fmadd(difference, amount, left_linear);
        rounded_byte(linear_to_srgb(premultiplied / alpha) * 255.0)
    };
    result.r = interpolate(left.r, right.r);
    result.g = interpolate(left.g, right.g);
    result.b = interpolate(left.b, right.b);
    result
}

pub fn average_gradient_color(stops: &[GradientStop]) -> Nrgba {
    if stops.is_empty() {
        return Nrgba::default();
    }
    let channels = |c: Nrgba| [c.r as f64, c.g as f64, c.b as f64, c.a as f64];
    let mut totals = [0.0f64; 4];
    let first = channels(stops[0].color);
    for channel in 0..4 {
        totals[channel] = go_fmadd(first[channel], stops[0].offset, totals[channel]);
    }
    for index in 1..stops.len() {
        let width = stops[index].offset - stops[index - 1].offset;
        let (left, right) = (
            channels(stops[index - 1].color),
            channels(stops[index].color),
        );
        for channel in 0..4 {
            // The product by 0.5 is exact, so its fusion is unobservable.
            totals[channel] += width * (left[channel] + right[channel]) / 2.0;
        }
    }
    let last = channels(stops[stops.len() - 1].color);
    for channel in 0..4 {
        totals[channel] = go_fmadd(
            last[channel],
            1.0 - stops[stops.len() - 1].offset,
            totals[channel],
        );
    }
    Nrgba::new(
        totals[0].round() as u8,
        totals[1].round() as u8,
        totals[2].round() as u8,
        totals[3].round() as u8,
    )
}

pub fn average_colrv1_gradient_color(stops: &[GradientStop]) -> Nrgba {
    if stops.is_empty() {
        return Nrgba::default();
    }
    let table = linear_srgb_byte();
    let components = |value: Nrgba| {
        let alpha = value.a as f64 / 255.0;
        [
            table[value.r as usize] * alpha,
            table[value.g as usize] * alpha,
            table[value.b as usize] * alpha,
            alpha,
        ]
    };
    let mut totals = [0.0f64; 4];
    let first = components(stops[0].color);
    for channel in 0..4 {
        totals[channel] = go_fmadd(first[channel], stops[0].offset, totals[channel]);
    }
    for index in 1..stops.len() {
        let width = stops[index].offset - stops[index - 1].offset;
        let (left, right) = (
            components(stops[index - 1].color),
            components(stops[index].color),
        );
        for channel in 0..4 {
            totals[channel] += width * (left[channel] + right[channel]) / 2.0;
        }
    }
    let last = components(stops[stops.len() - 1].color);
    for channel in 0..4 {
        totals[channel] = go_fmadd(
            last[channel],
            1.0 - stops[stops.len() - 1].offset,
            totals[channel],
        );
    }
    let mut result = Nrgba::new(0, 0, 0, rounded_byte(totals[3] * 255.0));
    if totals[3] <= 0.0 {
        return result;
    }
    result.r = rounded_byte(linear_to_srgb(totals[0] / totals[3]) * 255.0);
    result.g = rounded_byte(linear_to_srgb(totals[1] / totals[3]) * 255.0);
    result.b = rounded_byte(linear_to_srgb(totals[2] / totals[3]) * 255.0);
    result
}

pub fn rounded_byte(value: f64) -> u8 {
    if value <= 0.0 {
        return 0;
    }
    if value >= 255.0 {
        return 255;
    }
    value.round() as u8
}

// ------------------------------------------------------------ color_space.go

pub fn linear_srgb_byte() -> &'static [f64; 256] {
    static TABLE: std::sync::OnceLock<[f64; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0.0; 256];
        for (value, slot) in table.iter_mut().enumerate() {
            *slot = srgb_to_linear(value as f64 / 255.0);
        }
        table
    })
}

pub fn srgb_to_linear(value: f64) -> f64 {
    let value = go_max(0.0, go_min(1.0, value));
    if value <= 0.04045 {
        return value / 12.92;
    }
    gomath::pow((value + 0.055) / 1.055, 2.4)
}

pub fn premultiplied_srgb_byte_to_linear(channel: u8, alpha: u8) -> f64 {
    if alpha == 0 {
        return 0.0;
    }
    if alpha == 0xff {
        return linear_srgb_byte()[channel as usize];
    }
    srgb_to_linear(channel as f64 / alpha as f64)
}

pub fn linear_to_srgb(value: f64) -> f64 {
    let value = go_max(0.0, go_min(1.0, value));
    if value <= 0.0031308 {
        return 12.92 * value;
    }
    // ARM64 color_space.go:39: FNMSUBD.
    go_fnmsub(1.055, gomath::pow(value, 1.0 / 2.4), 0.055)
}

// ------------------------------------------------------- streamed coverage

/// compositeNRGBAOverRGBA: integer source-over of a straight-alpha sample.
#[inline]
pub fn composite_nrgba_over_rgba(destination: &mut [u8], source: Nrgba, coverage: u8) {
    if source.a == 0xff && coverage == 0xff {
        destination[0] = source.r;
        destination[1] = source.g;
        destination[2] = source.b;
        destination[3] = 0xff;
        return;
    }
    let mul255 = |a: u32, b: u32| (a * b + 127) / 255;
    let source_alpha = mul255(source.a as u32, coverage as u32);
    if destination[0] | destination[1] | destination[2] | destination[3] == 0 {
        destination[0] = mul255(source.r as u32, source_alpha) as u8;
        destination[1] = mul255(source.g as u32, source_alpha) as u8;
        destination[2] = mul255(source.b as u32, source_alpha) as u8;
        destination[3] = source_alpha as u8;
        return;
    }
    let inverse_alpha = 255 - source_alpha;
    for (channel, value) in [source.r, source.g, source.b].into_iter().enumerate() {
        let premultiplied = mul255(value as u32, source_alpha);
        let result = (premultiplied + mul255(destination[channel] as u32, inverse_alpha)).min(255);
        destination[channel] = result as u8;
    }
    let alpha = (source_alpha + mul255(destination[3] as u32, inverse_alpha)).min(255);
    destination[3] = alpha as u8;
}

/// drawRasterizedPaint: streams non-zero coverage into paint compositing.
pub fn draw_rasterized_paint(
    dst: &mut Rgba<'_>,
    bounds: Rect,
    paint: &PreparedPaint,
    scratch: &mut Scratch,
    purpose: &str,
    populate: &mut dyn FnMut(&mut Rasterizer, bool),
) -> Result<(), String> {
    if bounds.empty() {
        return Ok(());
    }
    paint.ensure_rendered(scratch)?;
    let region_filters = scratch.region_filters;
    let (rasterizer, budget) = scratch.reset_with_budget(Rect::new(0, 0, bounds.dx(), bounds.dy()));
    populate(rasterizer, region_filters);
    draw_paint_coverage(dst, bounds, rasterizer, budget, paint)
        .map_err(|e| format!("d2raster: {purpose}: {e}"))
}

fn walk_error(err: WalkError<()>) -> String {
    match err {
        WalkError::Scan(err) => err.to_string(),
        WalkError::Callback(()) => String::new(),
    }
}

fn draw_paint_coverage(
    dst: &mut Rgba<'_>,
    bounds: Rect,
    rasterizer: &mut Rasterizer,
    budget: &mut WorkBudget,
    paint: &PreparedPaint,
) -> Result<(), String> {
    let gradient = &*paint.gradient;
    let m = gradient.device_to_gradient;
    match paint.kind {
        PaintKind::Linear => walk_paint(dst, bounds, rasterizer, budget, |x, y| {
            let point = mpoint(&m, Point::new(x, y));
            // ARM64 gradient.go:873: the Y term is fused in the row loop.
            let parameter = go_fmadd(
                point.y - gradient.linear_start.y,
                gradient.linear_delta.y,
                (point.x - gradient.linear_start.x) * gradient.linear_delta.x,
            ) / gradient.linear_denominator;
            finite(parameter).then(|| gradient.sample_spread(parameter))
        }),
        PaintKind::LinearX | PaintKind::LinearY | PaintKind::LinearXOnly => {
            let (x_coefficient, y_coefficient, offset, start, delta) =
                axis_coefficients(paint.kind, gradient);
            walk_paint(dst, bounds, rasterizer, budget, |x, y| {
                // ARM64 gradient.go:828: the y term is fused.
                let coordinate = go_fmadd(y_coefficient, y, x_coefficient * x) + offset;
                let parameter = (coordinate - start) * delta / gradient.linear_denominator;
                finite(parameter).then(|| gradient.sample_spread(parameter))
            })
        }
        PaintKind::LinearYOnly => {
            let mut row_y = f64::NAN;
            let mut row_sample = None;
            walk_paint(dst, bounds, rasterizer, budget, |_, y| {
                if y.to_bits() != row_y.to_bits() {
                    row_y = y;
                    let coordinate = go_fmadd(m.d, y, m.f);
                    let parameter = (coordinate - gradient.linear_start.y)
                        * gradient.linear_delta.y
                        / gradient.linear_denominator;
                    row_sample = finite(parameter).then(|| gradient.sample_spread(parameter));
                }
                row_sample
            })
        }
        PaintKind::Radial => walk_paint(dst, bounds, rasterizer, budget, |x, y| {
            radial_sample(gradient, &m, x, y)
        }),
        PaintKind::Pattern => {
            if let Some(pattern) = paint.pattern.as_ref().filter(|p| p.direct_pixels) {
                let tile = pattern.tile_resource.image.borrow();
                if let Some(tile_image) = tile.as_ref() {
                    return walk_direct_pattern(
                        dst, bounds, rasterizer, budget, pattern, tile_image,
                    );
                }
            }
            walk_paint(dst, bounds, rasterizer, budget, |x, y| paint.color_at(x, y))
        }
        PaintKind::Solid => {
            walk_paint(dst, bounds, rasterizer, budget, |x, y| paint.color_at(x, y))
        }
    }
}

fn axis_coefficients(kind: PaintKind, gradient: &PreparedGradient) -> (f64, f64, f64, f64, f64) {
    let m = &gradient.device_to_gradient;
    match kind {
        PaintKind::LinearX => (
            m.a,
            m.c,
            m.e,
            gradient.linear_start.x,
            gradient.linear_delta.x,
        ),
        PaintKind::LinearY => (
            m.b,
            m.d,
            m.f,
            gradient.linear_start.y,
            gradient.linear_delta.y,
        ),
        _ => (
            m.a,
            0.0,
            m.e,
            gradient.linear_start.x,
            gradient.linear_delta.x,
        ),
    }
}

fn radial_sample(gradient: &PreparedGradient, m: &Matrix, x: f64, y: f64) -> Option<Nrgba> {
    // ARM64 gradient.go:957–958.
    let point = Point::new(
        go_fmadd(m.a, x, m.c * y) + m.e,
        go_fmadd(m.b, x, m.d * y) + m.f,
    );
    match gradient.radial_parameter(point) {
        Some(parameter) => Some(gradient.sample_spread(parameter)),
        None => {
            if gradient.spread == SpreadMethod::REPEAT && gradient.radial_tangent {
                let sample = gradient.radial_average;
                return (sample.a != 0).then_some(sample);
            }
            None
        }
    }
}

/// The shared row loop of drawGeneralPaintCoverage and the specialized
/// gradient loops: sample at pixel centers where coverage is non-zero.
fn walk_paint(
    dst: &mut Rgba<'_>,
    bounds: Rect,
    rasterizer: &mut Rasterizer,
    budget: &mut WorkBudget,
    mut sample_at: impl FnMut(f64, f64) -> Option<Nrgba>,
) -> Result<(), String> {
    rasterizer
        .walk_coverage(
            budget,
            |local_y, min_x, partial: &[f32], difference: &[f32]| -> Result<(), ()> {
                let dst_y = bounds.min.y + local_y;
                let dst_offset = dst.pix_offset(bounds.min.x + min_x, dst_y);
                let y = dst_y as f64 + 0.5;
                let mut winding = 0.0f32;
                for index in 0..partial.len() {
                    winding += difference[index];
                    let coverage = quantize_coverage(partial[index] + winding);
                    if coverage == 0 {
                        continue;
                    }
                    let dst_x = bounds.min.x + min_x + index as i64;
                    let Some(sample) = sample_at(dst_x as f64 + 0.5, y) else {
                        continue;
                    };
                    if sample.a == 0 {
                        continue;
                    }
                    let offset = dst_offset + index * 4;
                    composite_nrgba_over_rgba(&mut dst.pix[offset..offset + 4], sample, coverage);
                }
                Ok(())
            },
        )
        .map_err(walk_error)
}

fn wrapped_pixel(value: i64, offset: i64, period: i64) -> i64 {
    let mut value = value % period;
    if value < 0 {
        value += period;
    }
    if value >= period - offset {
        return value - (period - offset);
    }
    value + offset
}

fn direct_pattern_sample(
    tile_image: &RgbaBuf,
    pixel_x: i64,
    pixel_y: i64,
) -> (Option<Nrgba>, [u8; 4]) {
    let offset = ((pixel_y - tile_image.rect.min.y) as usize) * tile_image.stride
        + ((pixel_x - tile_image.rect.min.x) as usize) * 4;
    let source = [
        tile_image.pix[offset],
        tile_image.pix[offset + 1],
        tile_image.pix[offset + 2],
        tile_image.pix[offset + 3],
    ];
    let alpha = source[3];
    if alpha == 0 {
        return (None, source);
    }
    let sample = if alpha == 0xff {
        Nrgba::new(source[0], source[1], source[2], alpha)
    } else {
        Nrgba::new(
            ((source[0] as u32 * 0xffff / alpha as u32) >> 8) as u8,
            ((source[1] as u32 * 0xffff / alpha as u32) >> 8) as u8,
            ((source[2] as u32 * 0xffff / alpha as u32) >> 8) as u8,
            alpha,
        )
    };
    (Some(sample), source)
}

fn walk_direct_pattern(
    dst: &mut Rgba<'_>,
    bounds: Rect,
    rasterizer: &mut Rasterizer,
    budget: &mut WorkBudget,
    pattern: &PreparedPattern,
    tile_image: &RgbaBuf,
) -> Result<(), String> {
    let tile = &pattern.tile_resource;
    rasterizer
        .walk_coverage(
            budget,
            |local_y, min_x, partial: &[f32], difference: &[f32]| -> Result<(), ()> {
                let dst_y = bounds.min.y + local_y;
                let dst_offset = dst.pix_offset(bounds.min.x + min_x, dst_y);
                let mut pixel_x =
                    wrapped_pixel(bounds.min.x + min_x, pattern.pixel_offset_x, tile.width);
                let pixel_y = wrapped_pixel(dst_y, pattern.pixel_offset_y, tile.height);
                let mut winding = 0.0f32;
                for index in 0..partial.len() {
                    winding += difference[index];
                    let coverage = quantize_coverage(partial[index] + winding);
                    if coverage != 0 {
                        let (sample, source) = direct_pattern_sample(tile_image, pixel_x, pixel_y);
                        if let Some(sample) = sample {
                            let offset = dst_offset + index * 4;
                            let destination = &mut dst.pix[offset..offset + 4];
                            if source[3] == 0xff && coverage == 0xff {
                                destination
                                    .copy_from_slice(&[source[0], source[1], source[2], 0xff]);
                            } else {
                                composite_nrgba_over_rgba(destination, sample, coverage);
                            }
                        }
                    }
                    pixel_x += 1;
                    if pixel_x == tile.width {
                        pixel_x = 0;
                    }
                }
                Ok(())
            },
        )
        .map_err(walk_error)
}

/// drawPaintMask: paints through a populated Alpha coverage mask.
pub fn draw_paint_mask(
    dst: &mut Rgba<'_>,
    bounds: Rect,
    paint: &PreparedPaint,
    scratch: &mut Scratch,
    purpose: &str,
    populate: &mut dyn FnMut(&mut AlphaBuf) -> Result<(), String>,
) -> Result<(), String> {
    if bounds.empty() {
        return Ok(());
    }
    paint.ensure_rendered(scratch)?;
    let mask_bounds = Rect::new(0, 0, bounds.dx(), bounds.dy());
    let (mut mask, mask_bytes) = scratch.offscreen.new_alpha(mask_bounds, purpose)?;
    let result = (|| {
        populate(&mut mask)?;
        draw_mask_pixels(dst, bounds, &mask, paint);
        Ok(())
    })();
    scratch.offscreen.release(mask_bytes);
    result
}

fn draw_mask_pixels(dst: &mut Rgba<'_>, bounds: Rect, mask: &AlphaBuf, paint: &PreparedPaint) {
    let gradient = &*paint.gradient;
    let m = gradient.device_to_gradient;
    match paint.kind {
        PaintKind::Linear => mask_paint(dst, bounds, mask, |x, y| {
            let point = mpoint(&m, Point::new(x, y));
            // ARM64 gradient.go:1208: the Y term is fused.
            let parameter = go_fmadd(
                point.y - gradient.linear_start.y,
                gradient.linear_delta.y,
                (point.x - gradient.linear_start.x) * gradient.linear_delta.x,
            ) / gradient.linear_denominator;
            finite(parameter).then(|| gradient.sample_spread(parameter))
        }),
        PaintKind::LinearX | PaintKind::LinearY | PaintKind::LinearXOnly => {
            let (x_coefficient, y_coefficient, offset, start, delta) =
                axis_coefficients(paint.kind, gradient);
            mask_paint(dst, bounds, mask, |x, y| {
                // ARM64 gradient.go:1163.
                let coordinate = go_fmadd(y_coefficient, y, x_coefficient * x) + offset;
                let parameter = (coordinate - start) * delta / gradient.linear_denominator;
                finite(parameter).then(|| gradient.sample_spread(parameter))
            })
        }
        PaintKind::LinearYOnly => {
            let mut row_y = f64::NAN;
            let mut row_sample = None;
            mask_paint(dst, bounds, mask, |_, y| {
                if y.to_bits() != row_y.to_bits() {
                    row_y = y;
                    let coordinate = go_fmadd(m.d, y, m.f);
                    let parameter = (coordinate - gradient.linear_start.y)
                        * gradient.linear_delta.y
                        / gradient.linear_denominator;
                    row_sample = finite(parameter).then(|| gradient.sample_spread(parameter));
                }
                row_sample
            })
        }
        PaintKind::Radial => {
            mask_paint(dst, bounds, mask, |x, y| radial_sample(gradient, &m, x, y))
        }
        PaintKind::Pattern => {
            if let Some(pattern) = paint.pattern.as_ref().filter(|p| p.direct_pixels) {
                let tile = pattern.tile_resource.image.borrow();
                if let Some(tile_image) = tile.as_ref() {
                    direct_pattern_mask(dst, bounds, mask, pattern, tile_image);
                    return;
                }
            }
            mask_paint(dst, bounds, mask, |x, y| paint.color_at(x, y))
        }
        PaintKind::Solid => mask_paint(dst, bounds, mask, |x, y| paint.color_at(x, y)),
    }
}

fn mask_paint(
    dst: &mut Rgba<'_>,
    bounds: Rect,
    mask: &AlphaBuf,
    mut sample_at: impl FnMut(f64, f64) -> Option<Nrgba>,
) {
    let width = mask.rect.dx() as usize;
    for local_y in 0..mask.rect.dy() {
        let dst_y = bounds.min.y + local_y;
        let mask_offset = local_y as usize * mask.stride;
        let dst_offset = dst.pix_offset(bounds.min.x, dst_y);
        let y = dst_y as f64 + 0.5;
        for local_x in 0..width {
            let coverage = mask.pix[mask_offset + local_x];
            if coverage == 0 {
                continue;
            }
            let dst_x = bounds.min.x + local_x as i64;
            let Some(sample) = sample_at(dst_x as f64 + 0.5, y) else {
                continue;
            };
            if sample.a == 0 {
                continue;
            }
            let offset = dst_offset + local_x * 4;
            composite_nrgba_over_rgba(&mut dst.pix[offset..offset + 4], sample, coverage);
        }
    }
}

fn direct_pattern_mask(
    dst: &mut Rgba<'_>,
    bounds: Rect,
    mask: &AlphaBuf,
    pattern: &PreparedPattern,
    tile_image: &RgbaBuf,
) {
    let tile = &pattern.tile_resource;
    let mut pixel_y = wrapped_pixel(bounds.min.y, pattern.pixel_offset_y, tile.height);
    let width = mask.rect.dx() as usize;
    for local_y in 0..mask.rect.dy() {
        let dst_y = bounds.min.y + local_y;
        let mask_offset = local_y as usize * mask.stride;
        let dst_offset = dst.pix_offset(bounds.min.x, dst_y);
        let mut pixel_x = wrapped_pixel(bounds.min.x, pattern.pixel_offset_x, tile.width);
        for local_x in 0..width {
            let coverage = mask.pix[mask_offset + local_x];
            if coverage != 0 {
                let (sample, source) = direct_pattern_sample(tile_image, pixel_x, pixel_y);
                let offset = dst_offset + local_x * 4;
                let destination = &mut dst.pix[offset..offset + 4];
                if source[3] == 0xff && coverage == 0xff {
                    destination.copy_from_slice(&[source[0], source[1], source[2], 0xff]);
                } else if let Some(sample) = sample {
                    composite_nrgba_over_rgba(destination, sample, coverage);
                }
            }
            pixel_x += 1;
            if pixel_x == tile.width {
                pixel_x = 0;
            }
        }
        pixel_y += 1;
        if pixel_y == tile.height {
            pixel_y = 0;
        }
    }
}

// ----------------------------------------------------------------- pattern.go

#[derive(Debug)]
pub struct PreparedPattern {
    pub tile: d2_rs_scene::Box,
    pub origin_mod_x: f64,
    pub origin_mod_y: f64,
    pub device_to_pattern: Matrix,
    pub tile_resource: Rc<PatternTile>,
    pub direct_pixels: bool,
    pub pixel_offset_x: i64,
    pub pixel_offset_y: i64,
}

/// preparedPatternTile: one bounded tile rendered once per frame.
#[derive(Debug)]
pub struct PatternTile {
    pub width: i64,
    pub height: i64,
    pub bounds: Rect,
    pub tile_bytes: i64,
    pub root: RefCell<PreparedNode>,
    pub image: RefCell<Option<RgbaBuf>>,
    pub rendering: Cell<bool>,
}

pub fn direct_pattern_pixel_offsets(
    tile: d2_rs_scene::Box,
    device_to_pattern: &Matrix,
    width: i64,
    height: i64,
) -> (i64, i64, bool) {
    if width <= 0
        || height <= 0
        || tile.width != width as f64
        || tile.height != height as f64
        || device_to_pattern.a != 1.0
        || device_to_pattern.b != 0.0
        || device_to_pattern.c != 0.0
        || device_to_pattern.d != 1.0
    {
        return (0, 0, false);
    }
    let exact_integer_limit = (1u64 << 52) as f64;
    for value in [tile.x, tile.y, device_to_pattern.e, device_to_pattern.f] {
        if !finite(value)
            || value.trunc() != value
            || value < -exact_integer_limit
            || value > exact_integer_limit
        {
            return (0, 0, false);
        }
    }
    let offset = |translation: f64, origin: f64, period: i64| {
        let mut value = (translation as i64) % period - (origin as i64) % period;
        value %= period;
        if value < 0 {
            value += period;
        }
        value
    };
    (
        offset(device_to_pattern.e, tile.x, width),
        offset(device_to_pattern.f, tile.y, height),
        true,
    )
}

pub fn prepare_pattern_transform(
    units: PaintUnits,
    pattern_transform: &Matrix,
    object_bounds: d2_rs_scene::Box,
    object_to_device: &Matrix,
) -> Result<(Matrix, Matrix), String> {
    if units.0 > PaintUnits::USER_SPACE_ON_USE.0 {
        return Err(format!("invalid paint units {}", units.0));
    }
    if !pattern_transform.is_finite() {
        return Err("non-finite pattern transform".into());
    }
    if !object_to_device.is_finite() {
        return Err("non-finite object transform".into());
    }
    let units_to_object = units_to_object(units, object_bounds)?;
    let pattern_to_device = mmul(&mmul(object_to_device, &units_to_object), pattern_transform);
    if !pattern_to_device.is_finite() {
        return Err("composed pattern transform is non-finite".into());
    }
    match finite_affine_inverse(&pattern_to_device) {
        Ok(Some(inverse)) => Ok((pattern_to_device, inverse)),
        _ => Err("singular pattern transform".into()),
    }
}

pub fn pattern_tile_pixels(vector: Point) -> Result<i64, String> {
    let extent = gomath::hypot(vector.x, vector.y);
    if !finite(extent) || extent <= 0.0 {
        return Err("maps outside the finite pixel domain".into());
    }
    let extent = extent.ceil();
    if !finite(extent) || extent <= 0.0 || extent >= i64::MAX as f64 {
        return Err("exceeds the platform integer domain".into());
    }
    Ok(extent as i64)
}

impl PreparedPaint {
    pub fn ensure_rendered(&self, scratch: &mut Scratch) -> Result<(), String> {
        if self.kind != PaintKind::Pattern {
            return Ok(());
        }
        let Some(pattern) = &self.pattern else {
            return Err("d2raster: internal nil prepared pattern".into());
        };
        render_pattern_tile(&pattern.tile_resource, scratch)
    }
}

pub fn render_pattern_tile(tile: &Rc<PatternTile>, scratch: &mut Scratch) -> Result<(), String> {
    if tile.image.borrow().is_some() {
        return Ok(());
    }
    if tile.rendering.get() {
        return Err("d2raster: cyclic prepared pattern rendering".into());
    }
    let reservation = scratch
        .offscreen
        .reserve_bytes(tile.tile_bytes, "pattern tile")?;
    scratch.pattern_bytes += reservation;
    tile.rendering.set(true);
    let mut image_tile = RgbaBuf::new(tile.bounds);
    let result = {
        let root = tile.root.borrow();
        render_node(&mut image_tile.view(), &root, scratch)
    };
    tile.rendering.set(false);
    if let Err(err) = result {
        scratch.offscreen.release(reservation);
        scratch.pattern_bytes -= reservation;
        return Err(err);
    }
    *tile.image.borrow_mut() = Some(image_tile);
    scratch.pattern_tiles.push(Rc::clone(tile));
    Ok(())
}

impl PreparedPattern {
    pub fn color_at(&self, x: f64, y: f64) -> Option<Nrgba> {
        let tile = &self.tile_resource;
        let image = tile.image.borrow();
        let image = image.as_ref()?;
        let point = mpoint(&self.device_to_pattern, Point::new(x, y));
        let local_x = wrapped_pattern_coordinate_from_origin_mod(
            point.x,
            self.origin_mod_x,
            self.tile.width,
        )?;
        let local_y = wrapped_pattern_coordinate_from_origin_mod(
            point.y,
            self.origin_mod_y,
            self.tile.height,
        )?;
        let pixel_x = (local_x / self.tile.width * tile.width as f64).floor() as i64;
        let pixel_y = (local_y / self.tile.height * tile.height as f64).floor() as i64;
        let pixel_x = 0.max((tile.width - 1).min(pixel_x));
        let pixel_y = 0.max((tile.height - 1).min(pixel_y));
        direct_pattern_sample(image, pixel_x, pixel_y).0
    }
}

fn wrapped_pattern_coordinate_from_origin_mod(
    value: f64,
    origin_mod: f64,
    period: f64,
) -> Option<f64> {
    if !finite(value) || !finite(origin_mod) || !finite(period) || period <= 0.0 {
        return None;
    }
    let mut wrapped = value % period - origin_mod;
    if !finite(wrapped) {
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
