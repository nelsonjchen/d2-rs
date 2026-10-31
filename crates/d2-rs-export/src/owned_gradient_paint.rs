// SPDX-License-Identifier: MPL-2.0
// Source: d2raster/gradient.go prepareLinearGradient/prepareGradientTransform/colorAt.
use super::*;
use d2_rs_math::svg_primitive::{Paint, Stop};
pub(super) enum Prepared<'a> {
    Solid([u8; 4]),
    Linear {
        inverse: Affine64,
        start: [f64; 2],
        delta: [f64; 2],
        denominator: f64,
        stops: &'a [Stop],
        spread: u8,
        axis: u8,
    },
}
pub(super) fn prepare<'a>(
    paint: &'a Paint,
    g: &d2_rs_math::svg_geometry::Geometry,
    m: Affine64,
) -> Option<Prepared<'a>> {
    let Paint::Linear {
        start,
        end,
        stops,
        units,
        transform,
        spread,
    } = paint
    else {
        let Paint::Solid { color } = paint else {
            unreachable!()
        };
        return Some(Prepared::Solid(*color));
    };
    let mut units_matrix = [1., 0., 0., 1., 0., 0.];
    if *units == 0 {
        let b = match g.kind {
            1 => g.bounds,
            2 => [
                g.center[0] - g.radii[0],
                g.center[1] - g.radii[1],
                2. * g.radii[0],
                2. * g.radii[1],
            ],
            _ => {
                let p = d2_rs_math::svg_primitive::Primitive {
                    geometry: g.clone(),
                    fill: Some(Paint::Solid { color: [255; 4] }),
                    stroke: None,
                    fill_rule: 0,
                };
                let b = d2_rs_math::svg_bounds::primitive(&p, [1., 0., 0., 1., 0., 0.]).ok()?;
                [b.min[0], b.min[1], b.max[0] - b.min[0], b.max[1] - b.min[1]]
            }
        };
        if b[2] == 0. || b[3] == 0. {
            return None;
        }
        units_matrix = [b[2], 0., 0., b[3], b[0], b[1]];
    }
    let composed = source_affine_mul(
        source_affine_mul(m, affine(units_matrix)),
        affine(*transform),
    );
    let inverse = affine(d2_rs_math::svg_gradient::finite_inverse([
        composed.sx,
        composed.ky,
        composed.kx,
        composed.sy,
        composed.tx,
        composed.ty,
    ])?);
    let delta = [end[0] - start[0], end[1] - start[1]];
    let denominator = delta[0].mul_add(delta[0], delta[1] * delta[1]);
    if !denominator.is_finite() {
        return None;
    }
    if denominator == 0. || stops.len() == 1 {
        return Some(Prepared::Solid(stops.last()?.color));
    }
    let axis = if delta[1] == 0. && coordinate_safe(inverse.ky, inverse.sy, inverse.ty, start[1]) {
        1
    } else if delta[0] == 0. && coordinate_safe(inverse.sx, inverse.kx, inverse.tx, start[0]) {
        2
    } else {
        0
    };
    Some(Prepared::Linear {
        inverse,
        start: *start,
        delta,
        denominator,
        stops,
        spread: *spread,
        axis,
    })
}
impl Prepared<'_> {
    pub(super) fn color(&self, x: usize, y: usize) -> Option<[u8; 4]> {
        let Self::Linear {
            inverse,
            start,
            delta,
            denominator,
            stops,
            spread,
            axis,
        } = self
        else {
            let Self::Solid(c) = self else { unreachable!() };
            return Some(*c);
        };
        let (x, y) = (x as f64 + 0.5, y as f64 + 0.5);
        let t = if *axis == 1 {
            let px = if inverse.kx == 0. {
                inverse.sx.mul_add(x, inverse.tx)
            } else {
                inverse.sx.mul_add(x, inverse.kx * y) + inverse.tx
            };
            (px - start[0]) * delta[0] / denominator
        } else if *axis == 2 {
            let py = if inverse.ky == 0. {
                inverse.sy.mul_add(y, inverse.ty)
            } else {
                inverse.sy.mul_add(y, inverse.ky * x) + inverse.ty
            };
            (py - start[1]) * delta[1] / denominator
        } else {
            let px = inverse.sx.mul_add(x, inverse.kx * y) + inverse.tx;
            let py = inverse.ky.mul_add(x, inverse.sy * y) + inverse.ty;
            (px - start[0]).mul_add(delta[0], (py - start[1]) * delta[1]) / denominator
        };
        if !t.is_finite() {
            return None;
        }
        let t = spread_gradient_parameter(
            t,
            match spread {
                0 => usvg::SpreadMethod::Pad,
                1 => usvg::SpreadMethod::Reflect,
                _ => usvg::SpreadMethod::Repeat,
            },
        )?;
        let first = stops.first()?;
        let last = stops.last()?;
        if t < first.offset {
            return Some(first.color);
        }
        if t >= last.offset {
            return Some(last.color);
        }
        let pair = stops.windows(2).find(|s| t < s[1].offset)?;
        let a = (t - pair[0].offset) / (pair[1].offset - pair[0].offset);
        Some(std::array::from_fn(|i| {
            (f64::from(pair[1].color[i]) - f64::from(pair[0].color[i]))
                .mul_add(a, f64::from(pair[0].color[i]))
                .round() as u8
        }))
    }
    pub(super) fn paint(&self, raster: &mut Rasterizer, pixels: &mut [u8]) {
        match self {
            Self::Solid(c) => raster.paint(pixels, *c),
            _ => raster.paint_with_go_blend(pixels, |x, y| self.color(x, y)),
        }
    }
}

fn coordinate_safe(a: f64, b: f64, offset: f64, origin: f64) -> bool {
    let mut remaining = f64::MAX / 4.;
    for (factor, scale) in [
        (a, isize::MAX as f64),
        (b, isize::MAX as f64),
        (offset, 1.),
        (origin, 1.),
    ] {
        let magnitude = factor.abs();
        if magnitude > remaining / scale {
            return false;
        }
        remaining -= magnitude * scale;
    }
    true
}
