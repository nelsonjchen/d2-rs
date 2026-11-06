// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg svg_order.go: order complete source paint units by depth at
//! their actual projected intersections, keeping strongly connected
//! (mutually overlapping) units in their stable native order. No contraction
//! sites (products are explicitly rounded in Go).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use d2_rs_scene::{go_max, go_min};

use super::svg_visibility::{
    Budget, DepthPlane, Polygon, SVG_DEFAULT_VISIBILITY_LIMITS, SvgBox, SvgPoint,
    SvgVisibilityFace, SvgVisibilityLimits, bits_len, edge_distance, new_grid, polygon_area,
    polygon_box, polygon_plane, split_polygon,
};

/// The ordering view of an svgPaintUnit.
#[derive(Clone, Debug, Default)]
pub struct OrderUnit {
    /// Every visible polygon of every batch, in batch order.
    pub polygons: Vec<Vec<SvgPoint>>,
    pub first: i64,
    pub opaque: bool,
    pub depth: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrderStats {
    pub relations: usize,
    pub cyclic_groups: usize,
    pub conflicting_pairs: usize,
}

#[derive(Clone)]
struct OrderPolygon {
    points: Vec<SvgPoint>,
    plane: DepthPlane,
    bbox: SvgBox,
}

struct OrderTree {
    polygons: Vec<OrderPolygon>,
    indices: Vec<usize>,
    nodes: Vec<(SvgBox, usize, usize, i64, i64)>,
}

fn new_order_tree(polygons: Vec<OrderPolygon>, budget: &mut Budget) -> Result<OrderTree, String> {
    let mut tree = OrderTree {
        indices: (0..polygons.len()).collect(),
        polygons,
        nodes: Vec::new(),
    };
    fn build(
        tree: &mut OrderTree,
        start: usize,
        end: usize,
        budget: &mut Budget,
    ) -> Result<usize, String> {
        budget.spend(end - start + 1)?;
        let mut b = SvgBox::EMPTY;
        for &i in &tree.indices[start..end] {
            let p = tree.polygons[i].bbox;
            b.min_x = go_min(b.min_x, p.min_x);
            b.min_y = go_min(b.min_y, p.min_y);
            b.max_x = go_max(b.max_x, p.max_x);
            b.max_y = go_max(b.max_y, p.max_y);
        }
        let index = tree.nodes.len();
        tree.nodes.push((b, start, end, -1, -1));
        if end - start <= 8 {
            return Ok(index);
        }
        budget.spend((end - start) * bits_len(end - start))?;
        let x = b.max_x - b.min_x >= b.max_y - b.min_y;
        let polygons = &tree.polygons;
        tree.indices[start..end].sort_by(|&ai, &bi| {
            let (a, b) = (polygons[ai].bbox, polygons[bi].bbox);
            let (av, bv) = if x {
                (a.min_x + a.max_x, b.min_x + b.max_x)
            } else {
                (a.min_y + a.max_y, b.min_y + b.max_y)
            };
            if av == bv {
                ai.cmp(&bi)
            } else {
                av.partial_cmp(&bv).unwrap_or(Ordering::Equal)
            }
        });
        let middle = (start + end) / 2;
        let left = build(tree, start, middle, budget)?;
        let right = build(tree, middle, end, budget)?;
        tree.nodes[index].3 = left as i64;
        tree.nodes[index].4 = right as i64;
        Ok(index)
    }
    if !tree.polygons.is_empty() {
        let len = tree.polygons.len();
        build(&mut tree, 0, len, budget)?;
    }
    Ok(tree)
}

impl OrderTree {
    /// Depth-first visit of polygons overlapping `b`; `f` returns stop.
    fn visit(
        &self,
        b: &SvgBox,
        budget: &mut Budget,
        f: &mut dyn FnMut(&OrderPolygon, &mut Budget) -> Result<bool, String>,
    ) -> Result<bool, String> {
        fn walk(
            tree: &OrderTree,
            index: usize,
            b: &SvgBox,
            budget: &mut Budget,
            f: &mut dyn FnMut(&OrderPolygon, &mut Budget) -> Result<bool, String>,
        ) -> Result<bool, String> {
            budget.spend(1)?;
            let (node_box, start, end, left, right) = tree.nodes[index];
            if !b.overlaps(&node_box) {
                return Ok(false);
            }
            if left >= 0 {
                if walk(tree, left as usize, b, budget, f)? {
                    return Ok(true);
                }
                return walk(tree, right as usize, b, budget, f);
            }
            for &i in &tree.indices[start..end] {
                budget.spend(1)?;
                let p = &tree.polygons[i];
                if !b.overlaps(&p.bbox) {
                    continue;
                }
                if f(p, budget)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        if self.nodes.is_empty() {
            return Ok(false);
        }
        walk(self, 0, b, budget, f)
    }
}

/// 1 for a→b, 2 for b→a, 3 for a genuine crossing.
fn order_relation(
    a: &OrderTree,
    b: &OrderTree,
    source_a: i64,
    source_b: i64,
    budget: &mut Budget,
) -> Result<u8, String> {
    let swapped = a.polygons.len() > b.polygons.len();
    let (a, b, source_a, source_b) = if swapped {
        (b, a, source_b, source_a)
    } else {
        (a, b, source_a, source_b)
    };
    let mut relation = 0u8;
    for left in &a.polygons {
        let stop = b.visit(&left.bbox, budget, &mut |right, budget| {
            let next = polygon_relation(left, right, source_a, source_b, budget)?;
            relation |= next;
            Ok(relation == 3)
        })?;
        if stop {
            break;
        }
    }
    if swapped {
        relation = (relation & 1) << 1 | (relation & 2) >> 1;
    }
    Ok(relation)
}

fn polygon_relation(
    left: &OrderPolygon,
    right: &OrderPolygon,
    source_a: i64,
    source_b: i64,
    budget: &mut Budget,
) -> Result<u8, String> {
    let mut overlap = left.points.clone();
    let winding = if polygon_area(&right.points) < 0.0 {
        -1.0
    } else {
        1.0
    };
    for i in 0..right.points.len() {
        budget.spend(overlap.len() + 1)?;
        let distance = edge_distance(right.points[i], right.points[(i + 1) % right.points.len()]);
        let (inside, _) = split_polygon(&overlap, &|v| winding * distance(v));
        match inside.resolve(&overlap) {
            Some(points) => overlap = points,
            None => return Ok(0),
        }
    }
    let (mut low, mut high, mut magnitude) = (f64::INFINITY, f64::NEG_INFINITY, 1.0f64);
    for &p in &overlap {
        let (az, bz) = (left.plane.at(p), right.plane.at(p));
        low = go_min(low, bz - az);
        high = go_max(high, bz - az);
        magnitude = go_max(magnitude, go_max(az.abs(), bz.abs()));
    }
    let epsilon = go_max(1e-9, magnitude * 2e-14);
    if low >= -epsilon && high <= epsilon {
        return Ok(if source_a <= source_b { 1 } else { 2 });
    }
    let mut relation = 0;
    if high > epsilon {
        relation |= 1;
    }
    if low < -epsilon {
        relation |= 2;
    }
    Ok(relation)
}

/// svgOrderLess.
fn order_less(units: &[OrderUnit], a: usize, b: usize) -> bool {
    let (left, right) = (&units[a], &units[b]);
    if left.opaque != right.opaque {
        return left.opaque;
    }
    if !left.opaque && left.depth != right.depth {
        return left.depth < right.depth;
    }
    if left.first != right.first {
        return left.first < right.first;
    }
    a < b
}

fn order_cmp(units: &[OrderUnit], a: usize, b: usize) -> Ordering {
    if order_less(units, a, b) {
        Ordering::Less
    } else if order_less(units, b, a) {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// Iterative Kosaraju components.
fn order_components(
    edges: &[Vec<usize>],
    budget: &mut Budget,
) -> Result<(Vec<Vec<usize>>, Vec<usize>), String> {
    let mut reverse = vec![Vec::new(); edges.len()];
    for (from, list) in edges.iter().enumerate() {
        for &to in list {
            reverse[to].push(from);
        }
    }
    let mut seen = vec![false; edges.len()];
    let mut order = Vec::with_capacity(edges.len());
    for start in 0..edges.len() {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![(start, 0usize)];
        while let Some(top) = stack.last_mut() {
            budget.spend(1)?;
            if top.1 == edges[top.0].len() {
                order.push(top.0);
                stack.pop();
                continue;
            }
            let next = edges[top.0][top.1];
            top.1 += 1;
            if !seen[next] {
                seen[next] = true;
                stack.push((next, 0));
            }
        }
    }
    let mut membership = vec![usize::MAX; edges.len()];
    let mut components: Vec<Vec<usize>> = Vec::new();
    for &start in order.iter().rev() {
        if membership[start] != usize::MAX {
            continue;
        }
        let index = components.len();
        membership[start] = index;
        let mut stack = vec![start];
        let mut component = Vec::new();
        while let Some(n) = stack.pop() {
            budget.spend(1)?;
            component.push(n);
            for &next in &reverse[n] {
                if membership[next] == usize::MAX {
                    membership[next] = index;
                    stack.push(next);
                }
            }
        }
        components.push(component);
    }
    Ok((components, membership))
}

/// svgOrderPaintUnits: the painter order as a permutation of `units`.
pub fn order_paint_units(units: &[OrderUnit]) -> Result<Vec<usize>, String> {
    order_paint_units_with_limits(units, SVG_DEFAULT_VISIBILITY_LIMITS).map(|(order, _)| order)
}

pub fn order_paint_units_with_limits(
    units: &[OrderUnit],
    limits: SvgVisibilityLimits,
) -> Result<(Vec<usize>, OrderStats), String> {
    order_paint_units_with_context(units, limits, &Default::default())
}
pub fn order_paint_units_with_context(
    units: &[OrderUnit],
    limits: SvgVisibilityLimits,
    context: &super::context::RenderContext,
) -> Result<(Vec<usize>, OrderStats), String> {
    let mut stats = OrderStats::default();
    let mut budget = Budget {
        context: context.clone(),
        limits,
        work: 0,
        face: 0,
        faces: 0,
        candidates: 0,
        fragments: 0,
    };
    if units.len() > limits.faces {
        return Err("isometric SVG paint order exceeds unit limit".into());
    }
    let mut boxes = vec![Polygon::default(); units.len()];
    let mut faces = vec![SvgVisibilityFace::default(); units.len()];
    let mut trees = Vec::with_capacity(units.len());
    let (mut vertices, mut fragments) = (0usize, 0usize);
    for (i, unit) in units.iter().enumerate() {
        let mut b = SvgBox::EMPTY;
        let mut polygons = Vec::new();
        for points in &unit.polygons {
            budget.spend(points.len() + 1)?;
            let Some(plane) = polygon_plane(points) else {
                continue;
            };
            vertices += points.len();
            fragments += 1;
            if vertices > limits.vertices || fragments > limits.fragments {
                return Err("isometric SVG paint order exceeds polygon limit".into());
            }
            let bounds = polygon_box(points);
            polygons.push(OrderPolygon {
                points: points.clone(),
                plane,
                bbox: bounds,
            });
            b.min_x = go_min(b.min_x, bounds.min_x);
            b.min_y = go_min(b.min_y, bounds.min_y);
            b.max_x = go_max(b.max_x, bounds.max_x);
            b.max_y = go_max(b.max_y, bounds.max_y);
        }
        if !polygons.is_empty() {
            boxes[i] = Polygon {
                points: polygons[0].points.clone(),
                plane: polygons[0].plane,
                bbox: b,
                area: 0.0,
            };
            faces[i].opaque = true;
        }
        trees.push(polygons);
    }
    let trees: Vec<OrderTree> = trees
        .into_iter()
        .map(|p| new_order_tree(p, &mut budget))
        .collect::<Result<_, _>>()?;
    let grid = new_grid(&boxes, &faces, &mut budget)?;
    let mut edges = vec![Vec::new(); units.len()];
    let mut seen = vec![0usize; units.len()];
    for i in 0..boxes.len() {
        budget.spend(1)?;
        let Some(grid) = grid.as_ref() else { continue };
        if boxes[i].points.is_empty() {
            continue;
        }
        let mut visit = |candidates: &[usize], budget: &mut Budget| -> Result<(), String> {
            budget.spend(candidates.len())?;
            for &j in candidates {
                if j <= i || seen[j] == i + 1 {
                    continue;
                }
                seen[j] = i + 1;
                if !boxes[i].bbox.overlaps(&boxes[j].bbox) {
                    continue;
                }
                let relation =
                    order_relation(&trees[i], &trees[j], units[i].first, units[j].first, budget)?;
                if relation == 3 {
                    stats.conflicting_pairs += 1;
                }
                if relation & 1 != 0 {
                    edges[i].push(j);
                    stats.relations += 1;
                }
                if relation & 2 != 0 {
                    edges[j].push(i);
                    stats.relations += 1;
                }
                if stats.relations > limits.grid_references.min(1_000_000) {
                    return Err("isometric SVG paint order exceeds relation limit".into());
                }
            }
            Ok(())
        };
        if let Some((x0, y0, x1, y1)) = grid.cells_for(&boxes[i].bbox) {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    visit(&grid.cells[y * grid.nx + x], &mut budget)?;
                }
            }
            visit(&grid.large, &mut budget)?;
        }
    }
    let (mut components, membership) = order_components(&edges, &mut budget)?;
    for component in &mut components {
        if component.len() > 1 {
            stats.cyclic_groups += 1;
        }
        component.sort_by(|&a, &b| order_cmp(units, a, b));
    }
    let mut successors = vec![Vec::new(); components.len()];
    let mut indegree = vec![0usize; components.len()];
    for (from, list) in edges.iter().enumerate() {
        for &to in list {
            let (a, b) = (membership[from], membership[to]);
            if a == b {
                continue;
            }
            successors[a].push(b);
            indegree[b] += 1;
        }
    }
    // A min-heap on svgOrderLess of each component's first unit (a total
    // order, so container/heap's pop sequence is reproduced exactly).
    struct Ready<'a> {
        component: usize,
        units: &'a [OrderUnit],
        first: usize,
    }
    impl PartialEq for Ready<'_> {
        fn eq(&self, other: &Self) -> bool {
            self.cmp(other) == Ordering::Equal
        }
    }
    impl Eq for Ready<'_> {}
    impl PartialOrd for Ready<'_> {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Ready<'_> {
        fn cmp(&self, other: &Self) -> Ordering {
            order_cmp(self.units, other.first, self.first)
        }
    }
    let mut ready = BinaryHeap::new();
    for (i, n) in indegree.iter().enumerate() {
        if *n == 0 {
            ready.push(Ready {
                component: i,
                units,
                first: components[i][0],
            });
        }
    }
    let mut ordered = Vec::with_capacity(units.len());
    while let Some(Ready { component, .. }) = ready.pop() {
        budget.spend(1)?;
        ordered.extend_from_slice(&components[component]);
        for &to in &successors[component] {
            indegree[to] -= 1;
            if indegree[to] == 0 {
                ready.push(Ready {
                    component: to,
                    units,
                    first: components[to][0],
                });
            }
        }
    }
    if ordered.len() != units.len() {
        return Err("isometric SVG paint order has invalid component graph".into());
    }
    Ok((ordered, stats))
}
