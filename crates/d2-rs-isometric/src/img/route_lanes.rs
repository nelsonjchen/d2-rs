// SPDX-License-Identifier: MPL-2.0
//! route_lanes.go: bounded deterministic lane separation with local fan-outs.
use super::{
    geometry_routes::native_rounded_route,
    math as gomath,
    mesh::*,
    route_bridges::{Work, bridge_finite, bridge_straight_runs, bridge_visible},
    route_style::native_route_radius,
};
use crate::scene::{Board, Edge, Node};
use d2_rs_core::go_sort;
use d2_rs_render::go_float::go_fmadd;
use d2_rs_scene::{go_max, go_min};
use std::collections::{HashMap, HashSet};
const MAX_POINTS: usize = 100000;
const MIN_SHARED: f64 = 0.45;
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
struct Key {
    i: i64,
    k: i64,
    normal: i64,
    y: i64,
}
#[derive(Clone, Copy, Default)]
struct Interval {
    lo: f64,
    hi: f64,
}
#[derive(Clone)]
struct Segment {
    edge: usize,
    part: usize,
    a: Vec,
    b: Vec,
    u: Vec,
    normal: Vec,
    lo: f64,
    hi: f64,
    line: f64,
    length: f64,
    key: Key,
    shared: std::vec::Vec<Interval>,
}
#[derive(Clone)]
struct Obstacle {
    id: String,
    header: bool,
    left: f64,
    right: f64,
    front: f64,
    back: f64,
}
fn sense(s: &Segment) -> f64 {
    if ndot(nsub(s.b, s.a), s.u) < 0.0 {
        -1.0
    } else {
        1.0
    }
}
fn union(spans: &mut std::vec::Vec<Interval>, next: Interval) {
    if let Some(last) = spans.last_mut().filter(|p| next.lo <= p.hi + 0.4) {
        last.hi = go_max(last.hi, next.hi)
    } else {
        spans.push(next)
    }
}
fn lane_path(s: &Segment, offset: f64) -> std::vec::Vec<Vec> {
    let guard = 0.16;
    let taper = go_fmadd(offset.abs(), 0.7, 0.22);
    let mut intervals = vec![];
    let forward = ndot(nsub(s.b, s.a), s.u) > 0.0;
    for shared in &s.shared {
        let (mut lo, mut hi) = (shared.lo - s.lo, shared.hi - s.lo);
        if !forward {
            (lo, hi) = (s.length - hi, s.length - lo)
        }
        lo = go_max(guard + taper, lo);
        hi = go_min(s.length - guard - taper, hi);
        if hi < lo {
            continue;
        }
        intervals.push(Interval { lo, hi })
    }
    go_sort::sort_by(&mut intervals, |a, b| a.lo < b.lo);
    let mut merged: std::vec::Vec<Interval> = vec![];
    for span in intervals {
        if let Some(last) = merged
            .last_mut()
            .filter(|p| span.lo - p.hi < 2.0 * taper + 0.1)
        {
            last.hi = go_max(last.hi, span.hi)
        } else {
            merged.push(span)
        }
    }
    if merged.is_empty() {
        return vec![];
    }
    let mut out = vec![s.a];
    let mut add = |distance: f64, shift: f64| {
        let mut p = nadd(nlerp(s.a, s.b, distance / s.length), nmul(s.normal, shift));
        p.y = s.a.y;
        out.push(p)
    };
    for span in merged {
        add(span.lo - taper, 0.0);
        add(span.lo, offset);
        if span.hi > span.lo {
            add(span.hi, offset)
        }
        add(span.hi + taper, 0.0)
    }
    out.push(s.b);
    out
}
fn obstacles(nodes: &[Node], boards: &[Board]) -> Result<std::vec::Vec<Obstacle>, String> {
    let mut out = vec![];
    for n in nodes {
        if n.container || n.opacity <= 0.0 {
            continue;
        }
        if !bridge_finite(n.position.x)
            || !bridge_finite(n.position.z)
            || !bridge_finite(n.size.x)
            || !bridge_finite(n.size.z)
            || n.size.x < 0.0
            || n.size.z < 0.0
        {
            return Err(format!(
                "isometric lane obstacle {:?} has invalid bounds",
                n.id
            ));
        }
        out.push(Obstacle {
            id: n.id.clone(),
            header: false,
            left: (-0.5f64).mul_add(n.size.x, n.position.x) - 0.08,
            right: 0.5f64.mul_add(n.size.x, n.position.x) + 0.08,
            front: (-0.5f64).mul_add(n.size.z, n.position.z) - 0.17,
            back: 0.5f64.mul_add(n.size.z, n.position.z) + 0.17,
        })
    }
    for b in boards {
        if b.label.is_empty() {
            continue;
        }
        if !bridge_finite(b.position.x)
            || !bridge_finite(b.position.z)
            || !bridge_finite(b.size.x)
            || !bridge_finite(b.size.z)
            || !bridge_finite(b.header_depth)
            || b.size.x < 0.0
            || b.size.z < 0.0
            || b.header_depth < 0.0
        {
            return Err(format!(
                "isometric lane board {:?} has invalid bounds",
                b.id
            ));
        }
        out.push(Obstacle {
            id: b.id.clone(),
            header: true,
            left: (-0.5f64).mul_add(b.size.x, b.position.x) + 0.2,
            right: 0.5f64.mul_add(b.size.x, b.position.x) - 0.2,
            front: (-0.5f64).mul_add(b.size.z, b.position.z) + 0.07,
            back: (-0.5f64).mul_add(b.size.z, b.position.z) + b.header_depth,
        })
    }
    Ok(out)
}
fn hits_box(a: Vec, b: Vec, o: &Obstacle, padding: f64) -> bool {
    let (mut lo, mut hi) = (0.0, 1.0);
    for axis in [
        [a.x, b.x - a.x, o.left - padding, o.right + padding],
        [a.z, b.z - a.z, o.front - padding, o.back + padding],
    ] {
        if axis[1].abs() < 1e-12 {
            if axis[0] <= axis[2] + 1e-9 || axis[0] >= axis[3] - 1e-9 {
                return false;
            }
            continue;
        }
        let (mut x, mut y) = ((axis[2] - axis[0]) / axis[1], (axis[3] - axis[0]) / axis[1]);
        if x > y {
            std::mem::swap(&mut x, &mut y)
        }
        lo = go_max(lo, x);
        hi = go_min(hi, y);
        if hi - lo <= 1e-9 {
            return false;
        }
    }
    hi - lo > 1e-9
}
fn clear(
    w: &mut Work,
    points: &[Vec],
    obstacles: &[Obstacle],
    edge: &Edge,
) -> Result<bool, String> {
    let padding = 1.65f64.mul_add(native_route_radius(edge), 0.02);
    let source = edge.points.as_deref().unwrap_or_default();
    for o in obstacles {
        let (mut start_through, mut end_from) = (0, points.len());
        if !o.header && !points.is_empty() && !source.is_empty() {
            let inside = |p: Vec| {
                p.x > o.left - padding
                    && p.x < o.right + padding
                    && p.z > o.front - padding
                    && p.z < o.back + padding
            };
            if o.id == edge.source && points[0] == source[0] && inside(points[0]) {
                for i in 1..points.len() {
                    w.spend(1)?;
                    if !inside(points[i]) {
                        start_through = i;
                        break;
                    }
                }
            }
            if o.id == edge.target
                && points.last() == source.last()
                && inside(*points.last().unwrap())
            {
                for i in (0..points.len() - 1).rev() {
                    w.spend(1)?;
                    if !inside(points[i]) {
                        end_from = i + 1;
                        break;
                    }
                }
            }
        }
        for i in 1..points.len() {
            w.spend(1)?;
            if i <= start_through || i >= end_from {
                continue;
            }
            if hits_box(points[i - 1], points[i], o, padding) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
pub fn native_lane_routes(
    edges: &[Edge],
    nodes: &[Node],
    boards: &[Board],
) -> Result<std::vec::Vec<std::vec::Vec<Vec>>, String> {
    native_resolve_lane_routes(Some(&Default::default()), edges, nodes, boards, 25000000)
}
/// route_lanes.go:62–75: validate the context, then debit cooperative work.
pub fn native_resolve_lane_routes(
    context: Option<&super::context::RenderContext>,
    edges: &[Edge],
    nodes: &[Node],
    boards: &[Board],
    limit: usize,
) -> Result<std::vec::Vec<std::vec::Vec<Vec>>, String> {
    let context = context.ok_or("isometric route lanes require a context")?;
    let mut work = Work::with_stage_context(limit, "route lanes", context);
    if edges.len() > MAX_POINTS || nodes.len() > MAX_POINTS || boards.len() > MAX_POINTS {
        return Err("isometric route lanes exceed scene limit".into());
    }
    work.spend(edges.len() + nodes.len() + boards.len())?;
    let mut paths = vec![];
    let mut parts = vec![];
    let mut segments = vec![];
    let mut groups: HashMap<Key, std::vec::Vec<usize>> = HashMap::new();
    let mut parallel: HashMap<(i64, i64, i64), std::vec::Vec<usize>> = HashMap::new();
    let mut ids = HashSet::new();
    let mut total = 0;
    for (i, e) in edges.iter().enumerate() {
        if !ids.insert(&e.id) {
            return Err(format!(
                "isometric route lanes require unique edge IDs: {:?}",
                e.id
            ));
        }
        let points = e.points.as_deref().unwrap_or_default();
        if points.len() > 10000 || points.len() > MAX_POINTS - total {
            return Err("isometric route lanes exceed point limit".into());
        }
        total += points.len();
        work.spend(points.len())?;
        let mut clean = vec![];
        for &p in points {
            if !bridge_finite(p.x) || !bridge_finite(p.y) || !bridge_finite(p.z) {
                return Err(format!(
                    "isometric lane route {:?} has invalid coordinates",
                    e.id
                ));
            }
            if clean.last().is_none_or(|&last| nlen(nsub(p, last)) > 1e-9) {
                clean.push(p)
            }
        }
        let path = bridge_straight_runs(&clean);
        let mut part = vec![None; path.len().saturating_sub(1)];
        if bridge_visible(e) {
            for j in 1..path.len() {
                let (a, b) = (path[j - 1], path[j]);
                let length = gomath::hypot(b.x - a.x, b.z - a.z);
                if a.y != b.y || length < MIN_SHARED {
                    continue;
                }
                let mut u = nv((b.x - a.x) / length, 0.0, (b.z - a.z) / length);
                if u.x < -1e-10 || u.x.abs() <= 1e-10 && u.z < 0.0 {
                    u = nmul(u, -1.0)
                }
                let normal = nv(-u.z, 0.0, u.x);
                let line = ndot(a, normal);
                let key = Key {
                    i: (u.x * 1e8).round() as i64,
                    k: (u.z * 1e8).round() as i64,
                    normal: (line * 1e6).round() as i64,
                    y: (a.y * 1e6).round() as i64,
                };
                let (mut lo, mut hi) = (ndot(a, u), ndot(b, u));
                if lo > hi {
                    std::mem::swap(&mut lo, &mut hi)
                }
                let at = segments.len();
                part[j - 1] = Some(at);
                segments.push(Segment {
                    edge: i,
                    part: j - 1,
                    a,
                    b,
                    u,
                    normal,
                    lo,
                    hi,
                    line,
                    length,
                    key,
                    shared: vec![],
                });
                groups.entry(key).or_default().push(at);
                parallel.entry((key.i, key.k, key.y)).or_default().push(at)
            }
        }
        paths.push(path);
        parts.push(part);
    }
    let obstacles = obstacles(nodes, boards)?;
    let mut keys: std::vec::Vec<_> = groups.keys().copied().collect();
    keys.sort_by_key(|a| (a.i, a.k, a.y, a.normal));
    let mut offsets = vec![0.0; segments.len()];
    let mut processed = vec![false; segments.len()];
    let mut resolved = vec![vec![]; segments.len()];
    let mut shared_count = 0isize;
    for key in keys {
        let indices = groups.get_mut(&key).unwrap();
        go_sort::sort_by(indices, |&ai, &bi| {
            let (a, b) = (&segments[ai], &segments[bi]);
            a.lo < b.lo
                || a.lo == b.lo
                    && (edges[a.edge].id < edges[b.edge].id
                        || edges[a.edge].id == edges[b.edge].id && a.part < b.part)
        });
        let mut max_radius = 0.0;
        for (oi, &ai) in indices.iter().enumerate() {
            max_radius = go_max(max_radius, native_route_radius(&edges[segments[ai].edge]));
            for &bi in &indices[oi + 1..] {
                work.spend(1)?;
                let (a, b) = (&segments[ai], &segments[bi]);
                if b.lo >= a.hi - MIN_SHARED {
                    break;
                }
                let (lo, hi) = (go_max(a.lo, b.lo), go_min(a.hi, b.hi));
                if a.edge == b.edge
                    || hi - lo < MIN_SHARED
                    || (a.line - b.line).abs() > 1e-7
                    || a.a.y != b.a.y
                {
                    continue;
                }
                let old_count = a.shared.len() + b.shared.len();
                union(&mut segments[ai].shared, Interval { lo, hi });
                union(&mut segments[bi].shared, Interval { lo, hi });
                shared_count += segments[ai].shared.len() as isize
                    + segments[bi].shared.len() as isize
                    - old_count as isize;
                if shared_count > MAX_POINTS as isize {
                    return Err("isometric route lanes exceed overlap interval limit".into());
                }
            }
        }
        let step = 3.3f64.mul_add(max_radius, 0.035);
        go_sort::sort_by(indices, |&ai, &bi| {
            let (a, b) = (&segments[ai], &segments[bi]);
            edges[a.edge].id < edges[b.edge].id
                || edges[a.edge].id == edges[b.edge].id && a.part < b.part
        });
        for &si in indices.iter() {
            let s = &segments[si];
            if s.shared.is_empty() {
                processed[si] = true;
                continue;
            }
            for candidate in 0..=2 * (0.55 / step) as usize {
                let mut offset = candidate.div_ceil(2) as f64 * step;
                if candidate % 2 == 0 {
                    offset = -offset
                }
                offset *= sense(s);
                let mut available = true;
                for &oi in &parallel[&(key.i, key.k, key.y)] {
                    work.spend(1)?;
                    let o = &segments[oi];
                    if oi == si
                        || o.edge == s.edge
                        || go_min(o.hi, s.hi) - go_max(o.lo, s.lo) < MIN_SHARED
                    {
                        continue;
                    }
                    let gap = 1.65f64.mul_add(
                        native_route_radius(&edges[s.edge]) + native_route_radius(&edges[o.edge]),
                        0.035,
                    );
                    if o.key == key {
                        if processed[oi] && (offset - offsets[oi]).abs() < gap - 1e-9 {
                            available = false;
                            break;
                        }
                    } else {
                        let other = o.line + offsets[oi];
                        if offset != 0.0
                            && other > go_min(s.line, s.line + offset) - gap
                            && other < go_max(s.line, s.line + offset) + gap
                        {
                            available = false;
                            break;
                        }
                    }
                }
                if !available {
                    continue;
                }
                if offset == 0.0 {
                    break;
                }
                let path = lane_path(s, offset);
                if path.len() < 2 {
                    continue;
                }
                work.spend(path.len() * 8)?;
                if !clear(
                    &mut work,
                    &native_rounded_route(&path),
                    &obstacles,
                    &edges[s.edge],
                )? {
                    continue;
                }
                if path.len() > MAX_POINTS - total {
                    return Err("isometric route lanes exceed resolved point limit".into());
                }
                total += path.len();
                offsets[si] = offset;
                resolved[si] = path;
                break;
            }
            processed[si] = true;
        }
    }
    join_corners(
        &mut work,
        &paths,
        &parts,
        &segments,
        &offsets,
        &mut resolved,
        &obstacles,
        edges,
    )?;
    for (i, path) in paths.iter_mut().enumerate() {
        if path.len() < 2 {
            continue;
        }
        let mut out = vec![path[0]];
        for (j, si) in parts[i].iter().enumerate() {
            if let Some(si) = si.filter(|&si| !resolved[si].is_empty()) {
                out.extend_from_slice(&resolved[si][1..])
            } else {
                out.push(path[j + 1])
            }
        }
        if out.len() > 10000 {
            return Err("isometric lane route exceeds 10000 control points".into());
        }
        *path = out;
    }
    Ok(paths)
}
fn join_corners(
    w: &mut Work,
    paths: &[std::vec::Vec<Vec>],
    parts: &[std::vec::Vec<Option<usize>>],
    segments: &[Segment],
    offsets: &[f64],
    resolved: &mut [std::vec::Vec<Vec>],
    obstacles: &[Obstacle],
    edges: &[Edge],
) -> Result<(), String> {
    let key = |p: Vec| {
        [
            (p.x * 1e7).round() as i64,
            (p.y * 1e7).round() as i64,
            (p.z * 1e7).round() as i64,
        ]
    };
    let mut joints: HashMap<[i64; 3], std::vec::Vec<(usize, usize)>> = HashMap::new();
    for (i, path) in paths.iter().enumerate() {
        for j in 1..path.len().saturating_sub(1) {
            w.spend(1)?;
            if let (Some(ai), Some(bi)) = (parts[i][j - 1], parts[i][j]) {
                joints.entry(key(path[j])).or_default().push((ai, bi));
            }
        }
    }
    let mut order: std::vec::Vec<_> = (0..edges.len()).collect();
    go_sort::sort_by(&mut order, |&i, &j| edges[i].id < edges[j].id);
    for ei in order {
        for j in 1..paths[ei].len().saturating_sub(1) {
            w.spend(1)?;
            let (Some(ai), Some(bi)) = (parts[ei][j - 1], parts[ei][j]) else {
                continue;
            };
            if offsets[ai] == 0.0
                || offsets[bi] == 0.0
                || resolved[ai].len() < 4
                || resolved[bi].len() < 4
            {
                continue;
            }
            let (a, b) = (&segments[ai], &segments[bi]);
            if a.b != b.a
                || a.a.y != b.b.y
                || offsets[ai] * sense(a) * offsets[bi] * sense(b) <= 0.0
            {
                continue;
            }
            let touches = |s: &Segment, end: bool| {
                let high = (sense(s) > 0.0) == end;
                s.shared
                    .iter()
                    .any(|span| high && span.hi >= s.hi - 1e-7 || !high && span.lo <= s.lo + 1e-7)
            };
            w.spend(a.shared.len() + b.shared.len())?;
            if !touches(a, true) || !touches(b, false) {
                continue;
            }
            let mut ordered = true;
            for &(mut oa, mut ob) in &joints[&key(a.b)] {
                w.spend(1)?;
                if segments[oa].edge == ei {
                    continue;
                }
                if segments[oa].key == b.key && segments[ob].key == a.key {
                    std::mem::swap(&mut oa, &mut ob)
                }
                if segments[oa].key != a.key || segments[ob].key != b.key {
                    continue;
                }
                let d1 = (offsets[ai] - offsets[oa]) * sense(a);
                let d2 = (offsets[bi] - offsets[ob]) * sense(b);
                if d1 * d2 < -1e-9 {
                    ordered = false;
                    break;
                }
            }
            if !ordered {
                continue;
            }
            let (u, v) = (nunit(nsub(a.b, a.a)), nunit(nsub(b.b, b.a)));
            let det = (-u.z).mul_add(v.x, u.x * v.z);
            if det.abs() < 0.35 {
                continue;
            }
            let (p, q) = (
                nadd(a.b, nmul(a.normal, offsets[ai])),
                nadd(b.a, nmul(b.normal, offsets[bi])),
            );
            let delta = nsub(q, p);
            let mut m = nadd(p, nmul(u, (-delta.z).mul_add(v.x, delta.x * v.z) / det));
            m.y = a.b.y;
            if nlen(nsub(m, a.b)) > go_min(0.85, 3.0 * go_max(offsets[ai].abs(), offsets[bi].abs()))
            {
                continue;
            }
            let (left, right) = (&resolved[ai], &resolved[bi]);
            let (mut li, mut ri) = (left.len() - 2, 1);
            while li >= 1 && (ndot(nsub(left[li], a.a), a.normal) - offsets[ai]).abs() > 1e-7 {
                li -= 1
            }
            while ri < right.len() - 1
                && (ndot(nsub(right[ri], b.a), b.normal) - offsets[bi]).abs() > 1e-7
            {
                ri += 1
            }
            if li < 1
                || ri >= right.len() - 1
                || ndot(nsub(m, left[li]), u) < 0.03
                || ndot(nsub(right[ri], m), v) < 0.03
            {
                continue;
            }
            let candidate = [left[li - 1], left[li], m, right[ri], right[ri + 1]];
            if !clear(w, &native_rounded_route(&candidate), obstacles, &edges[ei])? {
                continue;
            }
            let mut l = left[..li + 1].to_vec();
            l.push(m);
            let mut r = vec![m];
            r.extend_from_slice(&right[ri..]);
            resolved[ai] = l;
            resolved[bi] = r;
        }
    }
    Ok(())
}
