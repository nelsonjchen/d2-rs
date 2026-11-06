// Copyright (c) 2018, 2020 Kiel University and others.
// Copyright (c) 2020 Kiel University and others.
// Copyright (c) 2026 The elk-go Authors.
// SPDX-License-Identifier: EPL-2.0
// elk-go v0.2.0/internal/rectpacking/provider.go: greedy width approximation.
use crate::ElkNode;

#[derive(Clone, Copy)]
struct Drawing {
    width: f64,
    height: f64,
    option: usize,
    x: f64,
    y: f64,
}

pub(super) fn width(
    nodes: &mut [ElkNode],
    spacing: f64,
    padding: [f64; 4],
    aspect: f64,
    goal: &str,
    shift: bool,
) -> f64 {
    if nodes.is_empty() {
        return 0.0;
    }
    nodes[0].x = 0.0;
    nodes[0].y = 0.0;
    let mut drawing = Drawing {
        width: nodes[0].width,
        height: nodes[0].height,
        option: 0,
        x: 0.0,
        y: 0.0,
    };
    for id in 1..nodes.len() {
        let last = &nodes[id - 1];
        let node = &nodes[id];
        let mut candidates = Vec::new();
        for option in 1..=4 {
            let (mut x, mut y) = (0.0, 0.0);
            let (mut width, mut height) = (drawing.width, drawing.height);
            match option {
                1 => {
                    x = last.x + last.width + spacing;
                    let mut closest = None;
                    for p in &nodes[..id] {
                        let bottom = p.y + p.height;
                        if x < p.x + p.width + spacing
                            && closest.is_none_or(|c| last.y - bottom < last.y - c)
                        {
                            closest = Some(bottom);
                        }
                    }
                    y = if shift {
                        closest.map(|v| v + spacing).unwrap_or(0.0)
                    } else {
                        last.y
                    };
                    width = width.max(x + node.width);
                    height = height.max(y + node.height);
                }
                2 => {
                    y = last.y + last.height + spacing;
                    let mut closest = None;
                    for p in &nodes[..id] {
                        let right = p.x + p.width;
                        if y < p.y + p.height + spacing
                            && closest.is_none_or(|c| last.x - right < last.x - c)
                        {
                            closest = Some(right);
                        }
                    }
                    x = if shift {
                        closest.map(|v| v + spacing).unwrap_or(0.0)
                    } else {
                        last.x
                    };
                    width = width.max(x + node.width);
                    height = height.max(y + node.height);
                }
                3 => {
                    x = drawing.width + spacing;
                    width = drawing.width + spacing + node.width;
                    height = drawing.height.max(node.height);
                }
                4 => {
                    y = drawing.height + spacing;
                    width = drawing.width.max(node.width);
                    height = drawing.height + spacing + node.height;
                }
                _ => unreachable!(),
            }
            candidates.push(Drawing {
                width,
                height,
                option,
                x,
                y,
            });
        }
        let horizontal = padding[1] + padding[3];
        let vertical = padding[0] + padding[2];
        // Preserve the source filter precedence and all exact ties.
        let metrics = match goal {
            "ASPECT_RATIO_DRIVEN" => [2, 1, 0],
            "AREA_DRIVEN" => [1, 0, 2],
            _ => [0, 1, 2],
        };
        for metric in metrics {
            if candidates.len() <= 1 {
                break;
            }
            let score = |v: Drawing| match metric {
                0 => (aspect / (v.width + horizontal)).min(1.0 / (v.height + vertical)),
                1 => (v.width + horizontal) * (v.height + vertical),
                _ => ((v.width + horizontal) / (v.height + vertical) - aspect).abs(),
            };
            let best = candidates
                .iter()
                .map(|&v| score(v))
                .reduce(|a, b| if metric == 0 { a.max(b) } else { a.min(b) })
                .unwrap();
            candidates.retain(|&v| score(v) == best);
        }
        let mut best = candidates[0];
        if candidates.len() == 2 {
            let a = candidates[0];
            let b = candidates[1];
            let below = |o| o == 2 || o == 4;
            let right = |o| o == 1 || o == 3;
            if below(a.option) && below(b.option) {
                best = if a.option == 4 { a } else { b };
            } else if right(a.option) && right(b.option) {
                best = if a.option == 3 { a } else { b };
            } else if (a.option == 1 || a.option == 2) && (b.option == 1 || b.option == 2) {
                let (right, below) = if b.option == 1 { (b, a) } else { (a, b) };
                let right_height =
                    (last.y + last.height).max(right.y + node.height) - last.y.min(right.y);
                let right_area = (right.x + node.width - last.x) * right_height;
                let below_width =
                    (last.x + last.width).max(below.x + node.width) - last.x.min(below.x);
                let below_area = below_width * (below.y + node.height - last.y);
                best = if right_area <= below_area {
                    right
                } else {
                    below
                };
            }
        }
        nodes[id].x = best.x;
        nodes[id].y = best.y;
        drawing = best;
    }
    drawing.width
}
