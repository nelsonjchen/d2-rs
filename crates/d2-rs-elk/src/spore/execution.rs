// Copyright (c) 2017 Kiel University and others; 2026 The elk-go Authors.
// SPDX-License-Identifier: EPL-2.0
// Pinned internal/spore/execution.go, including its subtree exclusion rules.
use super::*;
pub(super) fn grow(nodes: &mut [Node], tree: &Tree) -> bool {
    fn at(nodes: &mut [Node], t: &Tree, parent: usize, overlapped: &mut bool) {
        for &child in &t.children[parent] {
            let delta = nodes[parent].vertex.sub(nodes[parent].original);
            nodes[child].translate(delta);
            let factor = overlap(nodes[parent].rect, nodes[child].rect);
            if factor > 1. {
                *overlapped = true
            }
            let delta = nodes[child].original.sub(nodes[parent].original);
            let center = V::new(
                crate::js_trig::product_sum(delta.x, factor, nodes[parent].vertex.x),
                crate::js_trig::product_sum(delta.y, factor, nodes[parent].vertex.y),
            );
            let move_by = center.sub(nodes[child].vertex);
            nodes[child].translate(move_by);
            at(nodes, t, child, overlapped);
        }
    }
    let mut overlapped = false;
    at(nodes, tree, tree.root, &mut overlapped);
    overlapped
}
fn translate(nodes: &mut [Node], t: &Tree, n: usize, v: V) {
    nodes[n].translate(v);
    for &child in &t.children[n] {
        translate(nodes, t, child, v)
    }
}
fn min_sub(nodes: &[Node], t: &Tree, outside: usize, sub: usize, current: f64, v: V) -> f64 {
    let mut min = current;
    for &child in &t.children[sub] {
        let a = nodes[outside].rect;
        let b = nodes[child].rect;
        if touches(a, b) {
            if (fuzzy(b.p.x, a.end().x, 0.0001) == 0 && v.x < 0.)
                || (fuzzy(b.end().x, a.p.x, 0.0001) == 0 && v.x > 0.)
                || (fuzzy(b.p.y, a.end().y, 0.0001) == 0 && v.y < 0.)
                || (fuzzy(b.end().y, a.p.y, 0.0001) == 0 && v.y > 0.)
            {
                min = 0.;
                break;
            }
        } else {
            min = min.min(rect_distance(a, b, v));
        }
        min = min.min(min_sub(nodes, t, outside, child, min, v));
    }
    min
}
fn min_all(nodes: &[Node], t: &Tree, entire: usize, sub: usize, current: f64, v: V) -> f64 {
    let mut min = current.min(min_sub(nodes, t, entire, sub, current, v));
    for &child in &t.children[entire] {
        if child != sub {
            min = min.min(min_all(nodes, t, child, sub, min, v))
        }
    }
    min
}
pub(super) fn compact(nodes: &mut [Node], t: &Tree, orthogonal: bool) {
    fn at(nodes: &mut [Node], t: &Tree, parent: usize, orthogonal: bool) {
        for &child in &t.children[parent] {
            at(nodes, t, child, orthogonal)
        }
        for &child in &t.children[parent] {
            let mut v = nodes[parent].vertex.sub(nodes[child].vertex);
            let (a, b) = (nodes[parent].rect, nodes[child].rect);
            if orthogonal {
                if v.x.abs() >= v.y.abs() {
                    v.y = 0.;
                    if b.end().y > a.p.y && b.p.y < a.end().y {
                        let d = (a.p.x - b.end().x).max(b.p.x - a.end().x);
                        v = v.length_to(d)
                    }
                } else {
                    v.x = 0.;
                    if b.end().x > a.p.x && b.p.x < a.end().x {
                        let d = (a.p.y - b.end().y).max(b.p.y - a.end().y);
                        v = v.length_to(d)
                    }
                }
            } else {
                v = v.length_to(underlap(a, b))
            }
            let min = min_all(nodes, t, t.root, child, v.length(), v);
            #[cfg(feature = "diagnostic-traces")]
            if std::env::var_os("D2RS_SPORE_TRACE").is_some() {
                eprintln!(
                    "{}",
                    serde_json::json!({"trace":"spore-compaction","parent":parent+1,"child":child+1,"parent_vertex":{"X":nodes[parent].vertex.x,"Y":nodes[parent].vertex.y},"child_vertex":{"X":nodes[child].vertex.x,"Y":nodes[child].vertex.y},"parent_rect":{"X":a.p.x,"Y":a.p.y,"Width":a.size.x,"Height":a.size.y},"child_rect":{"X":b.p.x,"Y":b.p.y,"Width":b.size.x,"Height":b.size.y},"scaled":{"X":v.x,"Y":v.y},"length":v.length(),"minimum":min})
                );
            }
            v = v.length_to(min);
            #[cfg(feature = "diagnostic-traces")]
            if std::env::var_os("D2RS_SPORE_TRACE").is_some() {
                eprintln!(
                    "{}",
                    serde_json::json!({"trace":"spore-vector","parent":parent+1,"child":child+1,"vector":{"X":v.x,"Y":v.y}})
                );
            }

            translate(nodes, t, child, v);
        }
    }
    at(nodes, t, t.root, orthogonal)
}
