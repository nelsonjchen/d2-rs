// SPDX-License-Identifier: MPL-2.0
// Source: d2raster/geometry.go roundedRectSubpaths/ellipseSubpaths and raster.go drawPrimitive.
use super::*;
fn mix(base: f64, k: f64, r: f64) -> f64 {
    if cfg!(target_arch = "aarch64") {
        k.mul_add(r, base)
    } else {
        base + k * r
    }
}
pub(super) fn shape(g: &d2_rs_math::svg_geometry::Geometry) -> SvgPath {
    let p = |x, y| SvgPoint { x, y };
    let k = 0.5522847498307936;
    let c = match g.kind {
        1 => {
            let [x, y, w, h] = g.bounds;
            let [rx, ry] = g.radii;
            if rx == 0. || ry == 0. {
                vec![
                    SvgPathCommand::MoveTo(p(x, y)),
                    SvgPathCommand::LineTo(p(x + w, y)),
                    SvgPathCommand::LineTo(p(x + w, y + h)),
                    SvgPathCommand::LineTo(p(x, y + h)),
                    SvgPathCommand::Close,
                ]
            } else {
                vec![
                    SvgPathCommand::MoveTo(p(x + rx, y)),
                    SvgPathCommand::LineTo(p(x + w - rx, y)),
                    SvgPathCommand::CubicTo(
                        p(mix(x + w - rx, k, rx), y),
                        p(x + w, mix(y + ry, -k, ry)),
                        p(x + w, y + ry),
                    ),
                    SvgPathCommand::LineTo(p(x + w, y + h - ry)),
                    SvgPathCommand::CubicTo(
                        p(x + w, mix(y + h - ry, k, ry)),
                        p(mix(x + w - rx, k, rx), y + h),
                        p(x + w - rx, y + h),
                    ),
                    SvgPathCommand::LineTo(p(x + rx, y + h)),
                    SvgPathCommand::CubicTo(
                        p(mix(x + rx, -k, rx), y + h),
                        p(x, mix(y + h - ry, k, ry)),
                        p(x, y + h - ry),
                    ),
                    SvgPathCommand::LineTo(p(x, y + ry)),
                    SvgPathCommand::CubicTo(
                        p(x, mix(y + ry, -k, ry)),
                        p(mix(x + rx, -k, rx), y),
                        p(x + rx, y),
                    ),
                    SvgPathCommand::Close,
                ]
            }
        }
        2 => {
            let [x, y] = g.center;
            let [rx, ry] = g.radii;
            vec![
                SvgPathCommand::MoveTo(p(x + rx, y)),
                SvgPathCommand::CubicTo(
                    p(x + rx, mix(y, k, ry)),
                    p(mix(x, k, rx), y + ry),
                    p(x, y + ry),
                ),
                SvgPathCommand::CubicTo(
                    p(mix(x, -k, rx), y + ry),
                    p(x - rx, mix(y, k, ry)),
                    p(x - rx, y),
                ),
                SvgPathCommand::CubicTo(
                    p(x - rx, mix(y, -k, ry)),
                    p(mix(x, -k, rx), y - ry),
                    p(x, y - ry),
                ),
                SvgPathCommand::CubicTo(
                    p(mix(x, k, rx), y - ry),
                    p(x + rx, mix(y, -k, ry)),
                    p(x + rx, y),
                ),
                SvgPathCommand::Close,
            ]
        }
        _ => return commands(&g.commands),
    };
    let mut out = commands(&[]);
    out.commands = c;
    out
}
pub(super) fn draw_solid(
    p: &d2_rs_math::svg_primitive::Primitive,
    m: Affine64,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    origin: Point64,
) -> bool {
    let Some(paths) = collect_raw_subpaths(&shape(&p.geometry), flatten_tolerance64(m), m) else {
        return false;
    };
    if let Some(fill) = &p.fill {
        let Some(paint) = owned_gradient_paint::prepare(fill, &p.geometry, m) else {
            return false;
        };
        let mapped: Vec<_> = paths
            .iter()
            .cloned()
            .map(|p| map_centerline(p, m))
            .collect();
        if p.fill_rule == 1 {
            paint_even_odd_paths(pixels, width, height, &mapped, |x, y| paint.color(x, y));
        } else {
            let mut raster = Rasterizer::new(width, height, 0.375);
            raster.set_reference_origin(origin);
            for (points, _) in &mapped {
                raster.add_fill_polyline(points, true);
            }
            paint.paint(&mut raster, pixels);
        }
    }
    if let Some(stroke) = &p.stroke {
        let Some(paint) = owned_gradient_paint::prepare(&stroke.paint, &p.geometry, m) else {
            return false;
        };
        let cap = match stroke.cap {
            0 => usvg::LineCap::Butt,
            1 => usvg::LineCap::Round,
            _ => usvg::LineCap::Square,
        };
        let join = match stroke.join {
            0 => usvg::LineJoin::Miter,
            1 => usvg::LineJoin::Round,
            _ => usvg::LineJoin::Bevel,
        };
        let mut raster = Rasterizer::new(width, height, 0.375);
        raster.set_reference_origin(origin);
        for (points, closed) in paths {
            if let Some(dashes) = &stroke.dashes {
                for run in Rasterizer::dashed_centerline_f64(
                    &clean_stroke_points(&points, closed),
                    closed,
                    dashes,
                    stroke.dash_offset,
                ) {
                    raster.add_stroke_polyline(
                        &run.points,
                        m,
                        stroke.width,
                        run.closed,
                        join,
                        stroke.miter_limit,
                        cap,
                    );
                }
            } else {
                raster.add_stroke_polyline(
                    &points,
                    m,
                    stroke.width,
                    closed,
                    join,
                    stroke.miter_limit,
                    cap,
                );
            }
        }
        paint.paint(&mut raster, pixels);
    }
    true
}

pub(super) fn bounds(
    p: &d2_rs_math::svg_primitive::Primitive,
    m: [f64; 6],
    width: usize,
    height: usize,
) -> Option<[f64; 4]> {
    let paths = collect_raw_subpaths(
        &shape(&p.geometry),
        flatten_tolerance64(affine(m)),
        affine(m),
    )?;
    let mut result = None;
    if p.fill.is_some() {
        result = point_bounds(
            paths.iter().flat_map(|p| p.0.iter().copied()),
            m,
            1.,
            width,
            height,
        );
    }
    if let Some(stroke) = &p.stroke {
        let mut points = Vec::new();
        for (values, closed) in paths {
            if let Some(dashes) = &stroke.dashes {
                for run in Rasterizer::dashed_centerline_f64(
                    &clean_stroke_points(&values, closed),
                    closed,
                    dashes,
                    stroke.dash_offset,
                ) {
                    points.extend(run.points);
                }
            } else {
                points.extend(clean_stroke_points(&values, closed));
            }
        }
        let factor = if stroke.join == 0 {
            stroke.miter_limit.max(if stroke.cap == 2 {
                std::f64::consts::SQRT_2
            } else {
                1.
            })
        } else if stroke.cap == 2 {
            std::f64::consts::SQRT_2
        } else {
            1.
        };
        result = union(
            result,
            point_bounds(
                points.into_iter(),
                m,
                stroke.width * d2_rs_math::svg_bounds::max_scale(m) / 2. * factor + 1.,
                width,
                height,
            ),
        );
    }
    result
}
