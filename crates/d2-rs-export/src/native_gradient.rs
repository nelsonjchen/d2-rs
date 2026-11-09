// SPDX-License-Identifier: MPL-2.0
// d2raster/raster.go preparePath and gradient.go prepareGradientTransform.
use super::*;

pub(super) fn collect(
    group: &usvg::Group,
    sources: &RasterSourceMap<'_>,
    svg: &SvgImagePathSources,
    output: &mut HashMap<String, HashMap<String, crate::svg_precision::RawGradient>>,
) {
    for child in group.children() {
        match child {
            usvg::Node::Group(group) => collect(group, sources, svg, output),
            usvg::Node::Path(path) => {
                let paints = [
                    path.fill().map(|fill| fill.paint()),
                    path.stroke().map(|stroke| stroke.paint()),
                ];
                let ids = paints
                    .into_iter()
                    .enumerate()
                    .filter_map(|(role, paint)| {
                        let id = match paint? {
                            Paint::LinearGradient(g) => g.id(),
                            Paint::RadialGradient(g) => g.id(),
                            _ => return None,
                        };
                        // usvg gives each objectBoundingBox instantiation a
                        // new ID. Retain the source paint role before lowering.
                        Some((id, svg.native_gradient_id(path.id(), role).unwrap_or(id)))
                    })
                    .filter(|(_, id)| {
                        sources
                            .gradient_stops
                            .get(*id)
                            .is_some_and(|g| g.object_bounding_box && g.source_transform.is_some())
                    })
                    .collect::<Vec<_>>();
                if ids.is_empty() {
                    continue;
                }
                let Some(bounds) = bounds(path.id(), sources, svg) else {
                    continue;
                };
                if !bounds.valid || bounds.max.x <= bounds.min.x || bounds.max.y <= bounds.min.y {
                    continue;
                }
                let units = Affine64 {
                    sx: bounds.max.x - bounds.min.x,
                    sy: bounds.max.y - bounds.min.y,
                    tx: bounds.min.x,
                    ty: bounds.min.y,
                    kx: 0.0,
                    ky: 0.0,
                };
                let mut paints = HashMap::new();
                for (id, source_id) in ids {
                    let mut raw = sources.gradient_stops[source_id].clone();
                    let m = raw.source_transform.unwrap();
                    let local = source_affine_mul(
                        units,
                        Affine64 {
                            sx: m[0],
                            ky: m[1],
                            kx: m[2],
                            sy: m[3],
                            tx: m[4],
                            ty: m[5],
                        },
                    );
                    raw.native_transform =
                        Some([local.sx, local.ky, local.kx, local.sy, local.tx, local.ty]);
                    paints.insert(id.to_owned(), raw);
                }
                output.insert(path.id().to_owned(), paints);
            }
            _ => {}
        }
    }
}

fn bounds(
    id: &str,
    sources: &RasterSourceMap<'_>,
    svg: &SvgImagePathSources,
) -> Option<d2_rs_scene::Bounds> {
    use d2_rs_scene::{Path, PathCommand as C};
    let commands = if let Some(source) = sources.get(id) {
        use d2_rs_render::RasterPathCommand as R;
        source
            .commands
            .iter()
            .map(|command| match *command {
                R::MoveTo(p) => C::move_to(p.x, p.y),
                R::LineTo(p) => C::line_to(p.x, p.y),
                R::CubicTo(a, b, p) => C::cubic_to(a.x, a.y, b.x, b.y, p.x, p.y),
                R::Close => C::close(),
            })
            .collect()
    } else {
        use crate::svg_precision::SvgPathCommand as S;
        svg.raw_path(id)?
            .commands
            .iter()
            .map(|command| {
                Some(match *command {
                    S::MoveTo(p) => C::move_to(p.x, p.y),
                    S::LineTo(p) => C::line_to(p.x, p.y),
                    S::CubicTo(a, b, p) => C::cubic_to(a.x, a.y, b.x, b.y, p.x, p.y),
                    S::Close => C::close(),
                    _ => return None,
                })
            })
            .collect::<Option<Vec<_>>>()?
    };
    Path {
        commands,
        ..Path::default()
    }
    .geometry_bounds()
    .ok()
}
