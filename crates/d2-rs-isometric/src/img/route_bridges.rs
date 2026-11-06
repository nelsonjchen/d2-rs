// SPDX-License-Identifier: MPL-2.0
//! route_bridges.go: bounded crossing sweep and independent rounded routes.
use super::{
    geometry::native_paint, geometry_routes::native_rounded_route, math as gomath, mesh::*,
    route_style::native_route_radius,
};
use crate::scene::Edge;
use d2_rs_core::go_sort;
use d2_rs_render::go_float::go_fmadd;
use d2_rs_scene::{go_max, go_min};
pub const BRIDGE_MAX_POINTS: usize = 350000;
#[derive(Clone, Copy)]
struct Segment {
    edge: usize,
    part: usize,
    a: Vec,
    b: Vec,
    length: f64,
    min_x: f64,
    max_x: f64,
    min_z: f64,
    max_z: f64,
}
#[derive(Clone, Copy)]
struct Crossing {
    distance: f64,
    clearance: f64,
    under: usize,
    under_distance: f64,
}
#[derive(Clone, Copy)]
struct Span {
    first: f64,
    last: f64,
    half: f64,
    height: f64,
}
pub struct Work {
    remaining: usize,
    kind: &'static str,
    context: Option<super::context::RenderContext>,
}
impl Work {
    pub(crate) fn with_stage_context(
        remaining: usize,
        kind: &'static str,
        context: &super::context::RenderContext,
    ) -> Self {
        Self {
            remaining,
            kind,
            context: Some(context.clone()),
        }
    }
    fn with_context(remaining: usize, context: &super::context::RenderContext) -> Self {
        Self::with_stage_context(remaining, "crossing bridges", context)
    }
    pub fn spend(&mut self, n: usize) -> Result<(), String> {
        if let Some(context) = &self.context {
            context.check_string()?;
        }
        if n > self.remaining {
            return Err(format!("isometric {} exceed work limit", self.kind));
        }
        self.remaining -= n;
        Ok(())
    }
}
pub fn bridge_visible(e: &Edge) -> bool {
    e.opacity > 0.0
        && e.stroke_width > 0
        && (!e.stroke_explicit || native_paint(&e.stroke, "#657e9e").a > 0)
}
pub fn bridge_finite(v: f64) -> bool {
    v.is_finite() && v.abs() <= 1e9
}
pub fn bridge_straight_runs(points: &[Vec]) -> std::vec::Vec<Vec> {
    let mut out = vec![];
    for &p in points {
        while out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let (u, v) = (nsub(b, a), nsub(p, b));
            if ndot(u, v) <= 0.0 || nlen(ncross(u, v)) > 1e-12 * nlen(u) * nlen(v) {
                break;
            }
            out.pop();
        }
        out.push(p)
    }
    out
}
fn intersection(a: Segment, b: Segment) -> Option<(f64, f64)> {
    let (ax, az, bx, bz) = (a.b.x - a.a.x, a.b.z - a.a.z, b.b.x - b.a.x, b.b.z - b.a.z);
    let det = go_fmadd(-az, bx, ax * bz);
    if det.abs() < 0.75 * a.length * b.length {
        return None;
    }
    let (dx, dz) = (b.a.x - a.a.x, b.a.z - a.a.z);
    let t = go_fmadd(-dz, bx, dx * bz) / det;
    let u = go_fmadd(-dz, ax, dx * az) / det;
    (t > 1e-8 && t < 1.0 - 1e-8 && u > 1e-8 && u < 1.0 - 1e-8).then_some((t, u))
}
fn ease(t: f64) -> f64 {
    go_fmadd(-0.5, gomath::cos(std::f64::consts::PI * t), 0.5)
}
fn height(spans: &[Span], distance: f64) -> f64 {
    for s in spans {
        if distance < s.first - s.half || distance > s.last + s.half {
            continue;
        }
        let t = go_min(
            1.0,
            go_min(
                (distance - s.first + s.half) / s.half,
                (s.last + s.half - distance) / s.half,
            ),
        );
        let lo = 3.min((t * 4.0) as usize);
        let f = go_fmadd(t, 4.0, -(lo as f64));
        return s.height
            * go_fmadd(
                ease((lo + 1) as f64 / 4.0),
                f,
                ease(lo as f64 / 4.0) * (1.0 - f),
            );
    }
    0.0
}
pub fn native_bridge_routes(edges: &[Edge]) -> Result<std::vec::Vec<std::vec::Vec<Vec>>, String> {
    native_resolve_bridge_routes(Some(&Default::default()), edges, 20000000)
}
/// route_bridges.go:58–85: explicit cooperative context and work allowance.
pub fn native_resolve_bridge_routes(
    context: Option<&super::context::RenderContext>,
    edges: &[Edge],
    limit: usize,
) -> Result<std::vec::Vec<std::vec::Vec<Vec>>, String> {
    let context = context.ok_or("isometric crossing bridges require a context")?;
    if edges.len() > BRIDGE_MAX_POINTS {
        return Err("isometric crossing bridges exceed edge limit".into());
    }
    let mut work = Work::with_context(limit, context);
    work.spend(edges.len())?;
    let mut ids = std::collections::HashSet::new();
    let mut paths = vec![];
    let mut segments = vec![];
    let mut parts = vec![];
    let mut total = 0;
    for (i, e) in edges.iter().enumerate() {
        if !ids.insert(&e.id) {
            return Err(format!(
                "isometric crossing bridges require unique edge IDs: {:?}",
                e.id
            ));
        }
        let points = e.points.as_deref().unwrap_or_default();
        if points.len() > 10000 || points.len() > (BRIDGE_MAX_POINTS - total) / 8 {
            return Err("isometric crossing bridges exceed route point limit".into());
        }
        work.spend(points.len())?;
        for p in points {
            if !bridge_finite(p.x) || !bridge_finite(p.y) || !bridge_finite(p.z) {
                return Err(format!(
                    "isometric crossing bridge route {:?} has invalid coordinates",
                    e.id
                ));
            }
        }
        let path = bridge_straight_runs(&native_rounded_route(points));
        total += path.len();
        let mut part = vec![None; path.len().saturating_sub(1)];
        if bridge_visible(e) {
            for j in 1..path.len() {
                let (a, b) = (path[j - 1], path[j]);
                let length = gomath::hypot(b.x - a.x, b.z - a.z);
                if length < 0.42 || a.y != b.y {
                    continue;
                }
                part[j - 1] = Some(segments.len());
                segments.push(Segment {
                    edge: i,
                    part: j - 1,
                    a,
                    b,
                    length,
                    min_x: go_min(a.x, b.x),
                    max_x: go_max(a.x, b.x),
                    min_z: go_min(a.z, b.z),
                    max_z: go_max(a.z, b.z),
                })
            }
        }
        paths.push(path);
        parts.push(part);
    }
    let mut order: std::vec::Vec<_> = (0..segments.len()).collect();
    go_sort::sort_by(&mut order, |&i, &j| {
        let (a, b) = (segments[i], segments[j]);
        a.min_x < b.min_x
            || a.min_x == b.min_x
                && (edges[a.edge].id < edges[b.edge].id
                    || edges[a.edge].id == edges[b.edge].id && a.part < b.part)
    });
    let mut crossings = vec![vec![]; segments.len()];
    for (oi, &ai) in order.iter().enumerate() {
        let a = segments[ai];
        for &bi in &order[oi + 1..] {
            work.spend(1)?;
            let b = segments[bi];
            if b.min_x > a.max_x {
                break;
            }
            if a.edge == b.edge || a.max_z < b.min_z || b.max_z < a.min_z || a.a.y != b.a.y {
                continue;
            }
            let Some((at, bt)) = intersection(a, b) else {
                continue;
            };
            let (mut upper, mut lower, mut ut, mut lt) = (ai, bi, at, bt);
            if edges[a.edge].id < edges[b.edge].id {
                (upper, lower, ut, lt) = (bi, ai, bt, at)
            }
            let clearance =
                native_route_radius(&edges[a.edge]) + native_route_radius(&edges[b.edge]) + 0.05;
            let half = go_max(0.42, clearance * 3.0) / 2.0;
            let (u, l) = (segments[upper], segments[lower]);
            if ut * u.length < half + 0.12
                || (1.0 - ut) * u.length < half + 0.12
                || lt * l.length < 0.12
                || (1.0 - lt) * l.length < 0.12
            {
                continue;
            }
            crossings[upper].push(Crossing {
                distance: ut * u.length,
                clearance,
                under: lower,
                under_distance: lt * l.length,
            })
        }
    }
    go_sort::sort_by(&mut order, |&i, &j| {
        let (a, b) = (segments[i], segments[j]);
        edges[a.edge].id < edges[b.edge].id
            || edges[a.edge].id == edges[b.edge].id && a.part < b.part
    });
    let mut spans: std::vec::Vec<std::vec::Vec<Span>> = vec![vec![]; segments.len()];
    for &si in &order {
        work.spend(crossings[si].len() + 1)?;
        let cs = &mut crossings[si];
        go_sort::sort_by(cs, |a, b| {
            a.distance < b.distance
                || a.distance == b.distance
                    && edges[segments[a.under].edge].id < edges[segments[b.under].edge].id
        });
        for c in cs {
            work.spend(spans[c.under].len() + 1)?;
            let height = c.clearance + height(&spans[c.under], c.under_distance);
            let half = go_max(0.42, height * 3.0) / 2.0;
            if c.distance - half < 0.12 || c.distance + half > segments[si].length - 0.12 {
                continue;
            }
            let span = Span {
                first: c.distance,
                last: c.distance,
                half,
                height,
            };
            let ss = &mut spans[si];
            if let Some(p) = ss
                .last()
                .copied()
                .filter(|p| span.first - span.half <= p.last + p.half + 0.12)
            {
                let p = Span {
                    last: span.last,
                    height: go_max(p.height, span.height),
                    half: go_max(p.half, span.half),
                    ..p
                };
                if p.first - p.half < 0.12 || p.last + p.half > segments[si].length - 0.12 {
                    continue;
                }
                *ss.last_mut().unwrap() = p;
            } else {
                ss.push(span)
            }
        }
    }
    for (i, path) in paths.iter_mut().enumerate() {
        if path.len() < 2 {
            continue;
        }
        let mut out = vec![path[0]];
        for (j, si) in parts[i].iter().enumerate() {
            if let Some(si) = si {
                for s in &spans[*si] {
                    work.spend(10)?;
                    if total > BRIDGE_MAX_POINTS - 10 {
                        return Err("isometric crossing bridges exceed resolved point limit".into());
                    }
                    total += 10;
                    let segment = segments[*si];
                    let mut add = |d: f64, factor: f64| {
                        let mut p = nlerp(segment.a, segment.b, d / segment.length);
                        p.y = go_fmadd(s.height, factor, segment.a.y);
                        out.push(p)
                    };
                    for k in 0..=4 {
                        let t = k as f64 / 4.0;
                        add(go_fmadd(t, s.half, s.first - s.half), ease(t))
                    }
                    if s.last != s.first {
                        add(s.last, 1.0)
                    }
                    for k in 1..=4 {
                        let t = k as f64 / 4.0;
                        add(go_fmadd(t, s.half, s.last), ease(1.0 - t))
                    }
                }
            }
            out.push(path[j + 1])
        }
        *path = out;
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_work_checks_cancellation_after_start() {
        let token = d2_rs_core::CancellationToken::new();
        let context = super::super::context::RenderContext::from_token(&token);
        let mut work = Work::with_context(1, &context);
        work.spend(1).unwrap();
        token.cancel();
        // The caller's cause precedes exhaustion at every subsequent debit.
        assert_eq!(work.spend(1).unwrap_err(), "context canceled");
    }
}
