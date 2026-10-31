// Copyright (c) 2017 Kiel University and others; 2026 The elk-go Authors.
// SPDX-License-Identifier: EPL-2.0
// Pinned internal/spore/{triangulation,scanline}.go: ordered identity sets.
use super::*;
#[derive(Clone)]
struct Triangle {
    v: [usize; 3],
    center: V,
}
impl Triangle {
    fn new(a: usize, b: usize, c: usize, p: &[V]) -> Self {
        let (ab, ac, bc) = (p[b].sub(p[a]), p[c].sub(p[a]), p[c].sub(p[b]));
        let e = ab.y.mul_add(p[a].y + p[b].y, ab.x * (p[a].x + p[b].x));
        let f = ac.y.mul_add(p[a].y + p[c].y, ac.x * (p[a].x + p[c].x));
        let g = 2. * ab.cross(bc);
        let x = (-ab.y).mul_add(f, ac.y * e) / g;
        let y = (-ac.x).mul_add(e, ab.x * f) / g;
        Self {
            v: [a, b, c],
            center: V::new(x, y),
        }
    }
    fn edges(&self) -> [Edge; 3] {
        let [a, b, c] = self.v;
        [(a, b), (b, c), (c, a)]
    }
}
pub(super) fn add_edge(edges: &mut Vec<Edge>, e: Edge) {
    if !edges
        .iter()
        .any(|&(a, b)| (a == e.0 && b == e.1) || (a == e.1 && b == e.0))
    {
        edges.push(e)
    }
}
pub(super) fn triangulate(input: &[V]) -> Vec<Edge> {
    let mut p = input.to_vec();
    let (mut min, mut max) = (
        V::new(f64::INFINITY, f64::INFINITY),
        V::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    for v in &p {
        min.x = min.x.min(v.x);
        min.y = min.y.min(v.y);
        max.x = max.x.max(v.x);
        max.y = max.y.max(v.y);
    }
    let size = max.sub(min);
    let n = p.len();
    p.extend([
        V::new(min.x - 50., min.y - size.x - 50.),
        V::new(min.x - 50., max.y + size.x + 50.),
        V::new(max.x + size.y / 2. + 50., min.y + size.y / 2.),
    ]);
    let mut triangles = vec![Triangle::new(n, n + 1, n + 2, &p)];
    for added in 0..n {
        let invalid = triangles
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                (fuzzy(
                    t.center.distance(p[added]),
                    t.center.distance(p[t.v[0]]),
                    0.0001,
                ) < 0)
                    .then_some(i)
            })
            .collect::<Vec<_>>();
        let mut boundary = vec![];
        for &i in &invalid {
            for e in triangles[i].edges() {
                let shared = invalid.iter().any(|&j| {
                    j != i
                        && triangles[j]
                            .edges()
                            .iter()
                            .any(|&(a, b)| (a == e.0 && b == e.1) || (a == e.1 && b == e.0))
                });
                if !shared {
                    boundary.push(e)
                }
            }
        }
        triangles = triangles
            .into_iter()
            .enumerate()
            .filter_map(|(i, t)| (!invalid.contains(&i)).then_some(t))
            .collect();
        for (a, b) in boundary {
            let t = Triangle::new(added, a, b, &p);
            if !triangles
                .iter()
                .any(|old| t.v.iter().all(|v| old.v.contains(v)))
            {
                triangles.push(t)
            }
        }
    }
    let mut edges = vec![];
    for t in triangles {
        for e in t.edges() {
            add_edge(&mut edges, e)
        }
    }
    edges.into_iter().filter(|&(a, b)| a < n && b < n).collect()
}
pub(super) fn scanline(nodes: &[Node]) -> Vec<Edge> {
    let mut events = vec![];
    for n in 0..nodes.len() {
        events.extend([(n, true), (n, false)])
    }
    events.sort_by(|&(a, low_a), &(b, low_b)| {
        let y_a = if low_a {
            nodes[a].rect.p.y
        } else {
            nodes[a].rect.end().y
        };
        let y_b = if low_b {
            nodes[b].rect.p.y
        } else {
            nodes[b].rect.end().y
        };
        java_cmp(y_a, y_b).then_with(|| low_a.cmp(&low_b))
    });
    let mut active: Vec<usize> = vec![];
    let mut edges = vec![];
    for (n, low) in events {
        if !low {
            if let Some(i) = active.iter().position(|&x| x == n) {
                active.remove(i);
            }
            continue;
        }
        let cmp = |a: usize, b: usize| {
            java_cmp(nodes[a].rect.p.x, nodes[b].rect.p.x)
                .then_with(|| java_cmp(nodes[a].original.x, nodes[b].original.x))
                .then_with(|| java_cmp(nodes[a].original.y, nodes[b].original.y))
        };
        let i = active.partition_point(|&a| cmp(a, n) == std::cmp::Ordering::Less);
        active.insert(i, n);
        let mut found = false;
        for &other in &active {
            if other != n
                && fuzzy(nodes[n].rect.p.x, nodes[other].rect.end().x, 0.0001) < 0
                && fuzzy(nodes[other].rect.p.x, nodes[n].rect.end().x, 0.0001) < 0
            {
                add_edge(&mut edges, (n, other));
                found = true
            } else if found {
                break;
            }
        }
    }
    edges
}
pub(super) fn tree(
    nodes: &[Node],
    edges: &[Edge],
    root: usize,
    cost: impl Fn(Edge) -> f64,
) -> Tree {
    let mut weights = edges.iter().map(|&e| (e, cost(e))).collect::<Vec<_>>();
    weights.sort_by(|a, b| java_cmp(a.1, b.1));
    let mut parents = vec![None; nodes.len()];
    let mut children = vec![vec![]; nodes.len()];
    parents[root] = Some(root);
    while !weights.is_empty() {
        let next = weights.iter().enumerate().find_map(|(i, &((a, b), w))| {
            if w <= f64::INFINITY {
                match (parents[a].is_some(), parents[b].is_some()) {
                    (true, false) => Some((i, a, b)),
                    (false, true) => Some((i, b, a)),
                    _ => None,
                }
            } else {
                None
            }
        });
        let Some((i, parent, child)) = next else {
            break;
        };
        children[parent].push(child);
        parents[child] = Some(parent);
        weights.remove(i);
    }
    Tree { root, children }
}
