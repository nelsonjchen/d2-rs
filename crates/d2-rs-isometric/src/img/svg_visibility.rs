// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg svg_visibility.go, svg_visibility_tree.go and
//! svg_fragments.go: exact projected-depth visibility for SVG output. Every
//! product in the Go source is explicitly rounded (float64(...)), so no
//! contraction applies.

use std::collections::{BTreeMap, BTreeSet};

use d2_rs_scene::{go_max, go_min};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SvgPoint {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Go's math.Round: half away from zero.
pub fn go_round(x: f64) -> f64 {
    x.round()
}

/// svgProjectedPoint: canonical binary grids before clipping.
pub fn svg_projected_point(x: f64, y: f64, z: f64) -> SvgPoint {
    SvgPoint {
        x: go_round(x * (1u64 << 24) as f64) / (1u64 << 24) as f64,
        y: go_round(y * (1u64 << 24) as f64) / (1u64 << 24) as f64,
        z: go_round(z * (1u64 << 36) as f64) / (1u64 << 36) as f64,
    }
}

/// svgVisibilityFace; `group` and `owner` are pointer identities as ids.
#[derive(Clone, Debug, Default)]
pub struct SvgVisibilityFace {
    pub points: Vec<SvgPoint>,
    pub opaque: bool,
    pub group: Option<usize>,
    pub owner: Option<usize>,
    pub contour: bool,
    pub order: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct SvgVisibilityLimits {
    pub faces: usize,
    pub vertices: usize,
    pub fragments: usize,
    pub work: usize,
    pub grid_references: usize,
}

pub const RASTER_MAX_TRIANGLES: usize = 1_000_000;
pub const RASTER_MAX_WORK: usize = 500_000_000;
pub const RASTER_COORDINATE_LIMIT: f64 = 1e9;

pub const SVG_DEFAULT_VISIBILITY_LIMITS: SvgVisibilityLimits = SvgVisibilityLimits {
    faces: RASTER_MAX_TRIANGLES,
    vertices: 8_000_000,
    fragments: 2_000_000,
    work: 100_000_000,
    grid_references: 8_000_000,
};

pub(crate) struct Budget {
    pub(crate) context: super::context::RenderContext,
    pub(crate) limits: SvgVisibilityLimits,
    pub(crate) work: usize,
    pub(crate) face: usize,
    pub(crate) faces: usize,
    pub(crate) candidates: usize,
    pub(crate) fragments: usize,
}

impl Budget {
    pub(crate) fn spend(&mut self, n: usize) -> Result<(), String> {
        if n > self.limits.work - self.work {
            return Err(format!(
                "isometric SVG visibility exceeds work limit ({} faces, face {}, {} candidates, {} fragments)",
                self.faces, self.face, self.candidates, self.fragments
            ));
        }
        self.work += n;
        self.context.check_string()
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SvgBox {
    pub(crate) min_x: f64,
    pub(crate) min_y: f64,
    pub(crate) max_x: f64,
    pub(crate) max_y: f64,
}

impl SvgBox {
    pub(crate) const EMPTY: Self = Self {
        min_x: f64::INFINITY,
        min_y: f64::INFINITY,
        max_x: f64::NEG_INFINITY,
        max_y: f64::NEG_INFINITY,
    };

    pub(crate) fn overlaps(&self, c: &SvgBox) -> bool {
        self.min_x < c.max_x && c.min_x < self.max_x && self.min_y < c.max_y && c.min_y < self.max_y
    }
}

pub(crate) fn polygon_box(points: &[SvgPoint]) -> SvgBox {
    let mut b = SvgBox::EMPTY;
    for p in points {
        b.min_x = go_min(b.min_x, p.x);
        b.min_y = go_min(b.min_y, p.y);
        b.max_x = go_max(b.max_x, p.x);
        b.max_y = go_max(b.max_y, p.y);
    }
    b
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DepthPlane {
    pub(crate) origin: SvgPoint,
    pub(crate) dx: f64,
    pub(crate) dy: f64,
}

impl DepthPlane {
    pub(crate) fn at(&self, v: SvgPoint) -> f64 {
        self.origin.z + self.dx * (v.x - self.origin.x) + self.dy * (v.y - self.origin.y)
    }
}

pub(crate) fn polygon_area(points: &[SvgPoint]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    let o = points[0];
    for i in 1..points.len() - 1 {
        let (a, b) = (points[i], points[i + 1]);
        area += (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    }
    area / 2.0
}

pub(crate) fn polygon_plane(points: &[SvgPoint]) -> Option<DepthPlane> {
    if points.len() < 3 {
        return None;
    }
    let o = points[0];
    let (mut best, mut index) = (0.0f64, 0);
    for i in 1..points.len() - 1 {
        let (a, b) = (points[i], points[i + 1]);
        let d = (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
        if d.abs() > best.abs() {
            best = d;
            index = i;
        }
    }
    if best.abs() <= 1e-14 {
        return None;
    }
    let (a, b) = (points[index], points[index + 1]);
    Some(DepthPlane {
        origin: o,
        dx: ((a.z - o.z) * (b.y - o.y) - (b.z - o.z) * (a.y - o.y)) / best,
        dy: ((a.x - o.x) * (b.z - o.z) - (b.x - o.x) * (a.z - o.z)) / best,
    })
}

#[derive(Clone, Debug)]
pub(crate) struct Polygon {
    pub(crate) points: Vec<SvgPoint>,
    pub(crate) plane: DepthPlane,
    pub(crate) bbox: SvgBox,
    pub(crate) area: f64,
}

impl Default for Polygon {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            plane: DepthPlane::default(),
            bbox: SvgBox::EMPTY,
            area: 0.0,
        }
    }
}

pub(crate) struct Grid {
    pub(crate) bounds: SvgBox,
    pub(crate) nx: usize,
    pub(crate) ny: usize,
    pub(crate) cells: Vec<Vec<usize>>,
    pub(crate) large: Vec<usize>,
}

fn grid_cells_for(
    bounds: &SvgBox,
    nx: usize,
    ny: usize,
    b: &SvgBox,
) -> Option<(usize, usize, usize, usize)> {
    if !bounds.overlaps(b) {
        return None;
    }
    let x = |v: f64| {
        let cell = ((v - bounds.min_x) / (bounds.max_x - bounds.min_x) * nx as f64).floor();
        clamp_cell(cell, nx)
    };
    let y = |v: f64| {
        let cell = ((v - bounds.min_y) / (bounds.max_y - bounds.min_y) * ny as f64).floor();
        clamp_cell(cell, ny)
    };
    Some((x(b.min_x), y(b.min_y), x(b.max_x), y(b.max_y)))
}

/// max(0, min(n-1, int(v))) with Go's float-to-int conversion.
fn clamp_cell(v: f64, n: usize) -> usize {
    let i = go_int(v);
    i.clamp(0, n as i64 - 1) as usize
}

/// Go int(float64) on arm64 (FCVTZS): truncation, saturating, NaN → 0.
pub fn go_int(v: f64) -> i64 {
    v as i64
}

impl Grid {
    pub(crate) fn cells_for(&self, b: &SvgBox) -> Option<(usize, usize, usize, usize)> {
        grid_cells_for(&self.bounds, self.nx, self.ny, b)
    }
}

pub(crate) fn new_grid(
    polygons: &[Polygon],
    faces: &[SvgVisibilityFace],
    budget: &mut Budget,
) -> Result<Option<Grid>, String> {
    let mut bounds = SvgBox::EMPTY;
    let mut count = 0usize;
    for (i, p) in polygons.iter().enumerate() {
        budget.spend(1)?;
        if p.points.is_empty() || !faces[i].opaque {
            continue;
        }
        bounds.min_x = go_min(bounds.min_x, p.bbox.min_x);
        bounds.min_y = go_min(bounds.min_y, p.bbox.min_y);
        bounds.max_x = go_max(bounds.max_x, p.bbox.max_x);
        bounds.max_y = go_max(bounds.max_y, p.bbox.max_y);
        count += 1;
    }
    if count == 0 {
        return Ok(None);
    }
    let (w, h) = (bounds.max_x - bounds.min_x, bounds.max_y - bounds.min_y);
    let cells = go_max(1.0, go_min(65536.0, count as f64 / 4.0));
    let nx = go_int((cells * w / h).sqrt().ceil()).clamp(1, 256) as usize;
    let ny = go_int((cells / nx as f64).ceil()).clamp(1, 256) as usize;
    let mut grid = Grid {
        bounds,
        nx,
        ny,
        cells: vec![Vec::new(); nx * ny],
        large: Vec::new(),
    };
    let mut references = 0usize;
    for (i, p) in polygons.iter().enumerate() {
        budget.spend(1)?;
        if p.points.is_empty() || !faces[i].opaque {
            continue;
        }
        let (x0, y0, x1, y1) = grid.cells_for(&p.bbox).unwrap_or((0, 0, 0, 0));
        let mut n = (x1 - x0 + 1) * (y1 - y0 + 1);
        if n > 64 {
            grid.large.push(i);
            n = 1;
        } else {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    grid.cells[y * nx + x].push(i);
                }
            }
        }
        references += n;
        if references > budget.limits.grid_references {
            return Err("isometric SVG visibility exceeds spatial index limit".into());
        }
    }
    Ok(Some(grid))
}

/// svgSplitPolygon: cut a convex polygon into inside (distance ≥ 0) and
/// outside pieces. Returns references to the input when it lies wholly on one
/// side, as Go returns the same slice.
pub(crate) fn split_polygon(
    points: &[SvgPoint],
    distance: &dyn Fn(SvgPoint) -> f64,
) -> (Split, Split) {
    if points.is_empty() {
        return (Split::None, Split::None);
    }
    let distances: Vec<f64> = points.iter().map(|p| distance(*p)).collect();
    let positive = distances.iter().any(|d| *d > 0.0);
    let negative = distances.iter().any(|d| *d < 0.0);
    if !negative {
        return (Split::Same, Split::None);
    }
    if !positive {
        return (Split::None, Split::Same);
    }
    let (mut inside, mut outside) = (Vec::new(), Vec::new());
    let mut previous = points[points.len() - 1];
    let mut dp = distances[points.len() - 1];
    for (i, &current) in points.iter().enumerate() {
        let dc = distances[i];
        if (dp < 0.0 && dc > 0.0) || (dp > 0.0 && dc < 0.0) {
            let t = dp / (dp - dc);
            let p = SvgPoint {
                x: previous.x + t * (current.x - previous.x),
                y: previous.y + t * (current.y - previous.y),
                z: previous.z + t * (current.z - previous.z),
            };
            inside.push(p);
            outside.push(p);
        }
        if dc >= 0.0 {
            inside.push(current);
        }
        if dc <= 0.0 {
            outside.push(current);
        }
        previous = current;
        dp = dc;
    }
    (
        Split::from(valid_fragment(inside)),
        Split::from(valid_fragment(outside)),
    )
}

/// One side of a split: nothing, the unchanged input, or a new polygon.
pub(crate) enum Split {
    None,
    Same,
    New(Vec<SvgPoint>),
}

impl From<Option<Vec<SvgPoint>>> for Split {
    fn from(points: Option<Vec<SvgPoint>>) -> Self {
        points.map_or(Split::None, Split::New)
    }
}

impl Split {
    pub(crate) fn resolve(self, input: &[SvgPoint]) -> Option<Vec<SvgPoint>> {
        match self {
            Split::None => None,
            Split::Same => (!input.is_empty()).then(|| input.to_vec()),
            Split::New(points) => (!points.is_empty()).then_some(points),
        }
    }
}

/// svgValidFragment for callers outside this module.
pub(crate) fn valid_fragment_public(points: Vec<SvgPoint>) -> Option<Vec<SvgPoint>> {
    valid_fragment(points)
}

fn valid_fragment(points: Vec<SvgPoint>) -> Option<Vec<SvgPoint>> {
    let mut points = points;
    if points.len() < 3 {
        return None;
    }
    let same = |a: SvgPoint, b: SvgPoint| {
        let epsilon = go_max(
            1e-9,
            go_max(go_max(a.x.abs(), a.y.abs()), go_max(b.x.abs(), b.y.abs())) * 1e-15,
        );
        (a.x - b.x).abs() <= epsilon && (a.y - b.y).abs() <= epsilon
    };
    for i in 0..points.len() {
        if !same(points[i], points[(i + 1) % points.len()]) {
            continue;
        }
        let mut clean: Vec<SvgPoint> = Vec::with_capacity(points.len());
        for &point in &points {
            if clean.last().is_none_or(|last| !same(point, *last)) {
                clean.push(point);
            }
        }
        if clean.len() > 1 && same(clean[0], clean[clean.len() - 1]) {
            clean.pop();
        }
        points = clean;
        break;
    }
    if points.len() < 3 {
        return None;
    }
    let b = polygon_box(&points);
    let span = go_max(b.max_x - b.min_x, b.max_y - b.min_y);
    if polygon_area(&points).abs() <= go_max(span * 1e-8, span * span * 1e-13) {
        return None;
    }
    Some(points)
}

pub(crate) fn edge_distance(a: SvgPoint, b: SvgPoint) -> impl Fn(SvgPoint) -> f64 {
    move |p: SvgPoint| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

/// svgSubtractPolygon; the flag reports Go's identity case (the subject
/// returned unchanged because the cut misses it).
fn subtract_polygon(
    subject: &[SvgPoint],
    cut: &[SvgPoint],
    budget: &mut Budget,
) -> Result<(Vec<Vec<SvgPoint>>, bool), String> {
    let mut intersection = subject.to_vec();
    for i in 0..cut.len() {
        budget.spend(intersection.len())?;
        let (inside, _) = split_polygon(
            &intersection,
            &edge_distance(cut[i], cut[(i + 1) % cut.len()]),
        );
        match inside.resolve(&intersection) {
            Some(points) => intersection = points,
            None => return Ok((vec![subject.to_vec()], true)),
        }
    }
    let mut remaining = subject.to_vec();
    let mut visible = Vec::new();
    for i in 0..cut.len() {
        budget.spend(remaining.len())?;
        let (inside, outside) =
            split_polygon(&remaining, &edge_distance(cut[i], cut[(i + 1) % cut.len()]));
        let outside = outside.resolve(&remaining);
        let inside = inside.resolve(&remaining);
        if let Some(outside) = outside {
            visible.push(outside);
        }
        match inside {
            Some(points) => remaining = points,
            None => break,
        }
    }
    Ok((visible, false))
}

struct TreeNode {
    bbox: SvgBox,
    left: i64,
    right: i64,
    start: usize,
    end: usize,
}

struct Tree {
    indices: Vec<usize>,
    nodes: Vec<TreeNode>,
}

fn new_tree(
    polygons: &[Polygon],
    faces: &[SvgVisibilityFace],
    budget: &mut Budget,
) -> Result<Tree, String> {
    let indices: Vec<usize> = (0..polygons.len())
        .filter(|&i| !polygons[i].points.is_empty() && faces[i].opaque)
        .collect();
    let mut tree = Tree {
        indices,
        nodes: Vec::new(),
    };
    if tree.indices.is_empty() {
        return Ok(tree);
    }
    fn build(
        tree: &mut Tree,
        polygons: &[Polygon],
        start: usize,
        end: usize,
        budget: &mut Budget,
    ) -> Result<usize, String> {
        budget.spend(end - start)?;
        let mut bbox = SvgBox::EMPTY;
        let mut centers = SvgBox::EMPTY;
        for &id in &tree.indices[start..end] {
            let b = polygons[id].bbox;
            bbox.min_x = go_min(bbox.min_x, b.min_x);
            bbox.min_y = go_min(bbox.min_y, b.min_y);
            bbox.max_x = go_max(bbox.max_x, b.max_x);
            bbox.max_y = go_max(bbox.max_y, b.max_y);
            let (x, y) = ((b.min_x + b.max_x) / 2.0, (b.min_y + b.max_y) / 2.0);
            centers.min_x = go_min(centers.min_x, x);
            centers.min_y = go_min(centers.min_y, y);
            centers.max_x = go_max(centers.max_x, x);
            centers.max_y = go_max(centers.max_y, y);
        }
        let index = tree.nodes.len();
        tree.nodes.push(TreeNode {
            bbox,
            left: -1,
            right: -1,
            start,
            end,
        });
        if end - start <= 16 {
            return Ok(index);
        }
        let x_axis = centers.max_x - centers.min_x >= centers.max_y - centers.min_y;
        let center = |id: usize| {
            let b = polygons[id].bbox;
            if x_axis {
                b.min_x + b.max_x
            } else {
                b.min_y + b.max_y
            }
        };
        budget.spend((end - start) * bits_len(end - start))?;
        tree.indices[start..end].sort_by(|&a, &b| {
            let (ca, cb) = (center(a), center(b));
            if ca == cb {
                a.cmp(&b)
            } else {
                ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
            }
        });
        let middle = (start + end) / 2;
        let left = build(tree, polygons, start, middle, budget)?;
        let right = build(tree, polygons, middle, end, budget)?;
        tree.nodes[index].left = left as i64;
        tree.nodes[index].right = right as i64;
        Ok(index)
    }
    let len = tree.indices.len();
    build(&mut tree, polygons, 0, len, budget)?;
    Ok(tree)
}

impl Tree {
    fn query(
        &self,
        polygons: &[Polygon],
        b: &SvgBox,
        budget: &mut Budget,
    ) -> Result<Vec<usize>, String> {
        if self.nodes.is_empty() {
            return Ok(Vec::new());
        }
        let mut stack = vec![0usize];
        let mut result = Vec::new();
        while let Some(index) = stack.pop() {
            budget.spend(1)?;
            let node = &self.nodes[index];
            if !node.bbox.overlaps(b) {
                continue;
            }
            if node.left >= 0 {
                stack.push(node.right as usize);
                stack.push(node.left as usize);
                continue;
            }
            budget.spend(node.end - node.start)?;
            for &id in &self.indices[node.start..node.end] {
                if polygons[id].bbox.overlaps(b) {
                    result.push(id);
                }
            }
        }
        Ok(result)
    }
}

pub(crate) fn bits_len(n: usize) -> usize {
    (usize::BITS - n.leading_zeros()) as usize
}

/// svgVisibleFragmentSet: indexed visible pieces of one subject face.
struct FragmentSet {
    bounds: SvgBox,
    nx: usize,
    ny: usize,
    cells: Vec<BTreeSet<usize>>,
    large: BTreeSet<usize>,
    fragments: BTreeMap<usize, (Vec<SvgPoint>, SvgBox)>,
    next: usize,
    vertices: usize,
    max_fragments: usize,
    max_vertices: usize,
    references: usize,
}

impl FragmentSet {
    fn new(
        points: &[SvgPoint],
        budget: &mut Budget,
        max_fragments: usize,
        max_vertices: usize,
    ) -> Result<Self, String> {
        let b = polygon_box(points);
        let (w, h) = (b.max_x - b.min_x, b.max_y - b.min_y);
        let nx = go_int((256.0 * w / h).sqrt().ceil()).clamp(1, 32) as usize;
        let ny = go_int((256.0 / nx as f64).ceil()).clamp(1, 32) as usize;
        let mut set = Self {
            bounds: b,
            nx,
            ny,
            cells: vec![BTreeSet::new(); nx * ny],
            large: BTreeSet::new(),
            fragments: BTreeMap::new(),
            next: 0,
            vertices: 0,
            max_fragments,
            max_vertices,
            references: 0,
        };
        set.add(points.to_vec(), budget)?;
        Ok(set)
    }

    fn change_cells(
        &mut self,
        id: usize,
        b: &SvgBox,
        add: bool,
        budget: &mut Budget,
    ) -> Result<(), String> {
        let Some((x0, y0, x1, y1)) = grid_cells_for(&self.bounds, self.nx, self.ny, b) else {
            return Ok(());
        };
        let mut count = (x1 - x0 + 1) * (y1 - y0 + 1);
        let large = count > 64;
        if large {
            count = 1;
        }
        budget.spend(count)?;
        if add {
            if count > budget.limits.grid_references - self.references {
                return Err("isometric SVG visibility exceeds fragment spatial index limit".into());
            }
            self.references += count;
        } else {
            self.references -= count;
        }
        if large {
            if add {
                self.large.insert(id);
            } else {
                self.large.remove(&id);
            }
            return Ok(());
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let cell = &mut self.cells[y * self.nx + x];
                if add {
                    cell.insert(id);
                } else {
                    cell.remove(&id);
                }
            }
        }
        Ok(())
    }

    fn add(&mut self, points: Vec<SvgPoint>, budget: &mut Budget) -> Result<(), String> {
        budget.spend(points.len() + 1)?;
        if self.fragments.len() >= self.max_fragments
            || points.len() > self.max_vertices - self.vertices
        {
            return Err("isometric SVG visibility exceeds fragment limit".into());
        }
        let b = polygon_box(&points);
        self.change_cells(self.next + 1, &b, true, budget)?;
        self.next += 1;
        self.vertices += points.len();
        self.fragments.insert(self.next, (points, b));
        Ok(())
    }

    fn query(&self, b: &SvgBox, budget: &mut Budget) -> Result<Vec<usize>, String> {
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        let mut visit = |bucket: &BTreeSet<usize>, budget: &mut Budget| -> Result<(), String> {
            budget.spend(bucket.len())?;
            for &id in bucket {
                if !seen.insert(id) {
                    continue;
                }
                if self.fragments[&id].1.overlaps(b) {
                    result.push(id);
                }
            }
            Ok(())
        };
        if let Some((x0, y0, x1, y1)) = grid_cells_for(&self.bounds, self.nx, self.ny, b) {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    visit(&self.cells[y * self.nx + x], budget)?;
                }
            }
            visit(&self.large, budget)?;
        }
        budget.spend(result.len() * bits_len(result.len()))?;
        result.sort_unstable();
        Ok(result)
    }

    fn subtract(&mut self, cut: &[SvgPoint], budget: &mut Budget) -> Result<(), String> {
        let ids = self.query(&polygon_box(cut), budget)?;
        for id in ids {
            let (points, b) = self.fragments[&id].clone();
            let (pieces, unchanged) = subtract_polygon(&points, cut, budget)?;
            if unchanged {
                continue;
            }
            self.change_cells(id, &b, false, budget)?;
            self.fragments.remove(&id);
            self.vertices -= points.len();
            for piece in pieces {
                self.add(piece, budget)?;
            }
        }
        Ok(())
    }

    fn polygons(self, budget: &mut Budget) -> Result<Vec<Vec<SvgPoint>>, String> {
        budget.spend(self.fragments.len() * bits_len(self.fragments.len()))?;
        Ok(self
            .fragments
            .into_values()
            .map(|(points, _)| points)
            .collect())
    }
}

/// svgVisibilityWorkLimit.
pub fn svg_visibility_work_limit(faces: usize) -> usize {
    if faces > RASTER_MAX_WORK / 2304 {
        return RASTER_MAX_WORK;
    }
    SVG_DEFAULT_VISIBILITY_LIMITS.work.max(faces * 2304)
}

/// svgVisibleFaces: the visible convex fragments of every face.
pub fn svg_visible_faces(faces: &[SvgVisibilityFace]) -> Result<Vec<Vec<Vec<SvgPoint>>>, String> {
    let mut limits = SVG_DEFAULT_VISIBILITY_LIMITS;
    limits.work = svg_visibility_work_limit(faces.len());
    svg_visible_faces_with_limits(faces, limits)
}

pub fn svg_visible_faces_with_limits(
    faces: &[SvgVisibilityFace],
    limits: SvgVisibilityLimits,
) -> Result<Vec<Vec<Vec<SvgPoint>>>, String> {
    svg_visible_faces_with_context(faces, limits, &Default::default())
}
pub fn svg_visible_faces_with_context(
    faces: &[SvgVisibilityFace],
    limits: SvgVisibilityLimits,
    context: &super::context::RenderContext,
) -> Result<Vec<Vec<Vec<SvgPoint>>>, String> {
    context.check_string()?;
    if faces.len() > limits.faces {
        return Err("isometric SVG visibility exceeds face limit".into());
    }
    let mut budget = Budget {
        context: context.clone(),
        limits,
        work: 0,
        face: 0,
        faces: faces.len(),
        candidates: 0,
        fragments: 0,
    };
    let mut polygons = vec![Polygon::default(); faces.len()];
    let mut input_vertices = 0;
    for (i, face) in faces.iter().enumerate() {
        budget.spend(face.points.len() + 1)?;
        input_vertices += face.points.len();
        if input_vertices > limits.vertices {
            return Err("isometric SVG visibility exceeds vertex limit".into());
        }
        for p in &face.points {
            if p.x.is_nan()
                || p.y.is_nan()
                || p.z.is_nan()
                || p.x.abs() > RASTER_COORDINATE_LIMIT
                || p.y.abs() > RASTER_COORDINATE_LIMIT
                || p.z.abs() > RASTER_COORDINATE_LIMIT
            {
                return Err(format!(
                    "isometric SVG visibility face {i} has invalid coordinates"
                ));
            }
        }
        let valid = valid_fragment(face.points.clone()).unwrap_or_default();
        let Some(plane) = polygon_plane(&valid) else {
            continue;
        };
        let mut points = valid;
        if polygon_area(&points) < 0.0 {
            points.reverse();
        }
        let bbox = polygon_box(&points);
        let area = go_round(polygon_area(&points) * 1e4) / 1e4;
        polygons[i] = Polygon {
            points,
            plane,
            bbox,
            area,
        };
    }
    let (mut grid, mut tree) = (None, None);
    if faces.len() > 8192 {
        tree = Some(new_tree(&polygons, faces, &mut budget)?);
    } else {
        grid = new_grid(&polygons, faces, &mut budget)?;
    }
    let mut output = vec![Vec::new(); faces.len()];
    let mut seen = vec![0usize; faces.len()];
    let (mut output_vertices, mut output_fragments) = (0usize, 0usize);
    for index in 0..polygons.len() {
        budget.face = index;
        budget.candidates = 0;
        budget.fragments = 1;
        budget.spend(1)?;
        let subject = &polygons[index];
        if subject.points.is_empty() {
            continue;
        }
        let mut fragments = vec![subject.points.clone()];
        let mut candidates = Vec::new();
        let mut visit = |indices: &[usize], budget: &mut Budget| -> Result<(), String> {
            budget.spend(indices.len())?;
            for &other in indices {
                if other == index || seen[other] == index + 1 {
                    continue;
                }
                seen[other] = index + 1;
                if faces[other].group.is_some() && faces[other].group != faces[index].group {
                    continue;
                }
                if faces[other].contour
                    && !faces[index].contour
                    && ((faces[index].owner.is_some() && faces[index].owner == faces[other].owner)
                        || (faces[index].group.is_some()
                            && faces[index].group == faces[other].group))
                {
                    continue;
                }
                if subject.bbox.overlaps(&polygons[other].bbox) {
                    candidates.push(other);
                }
            }
            Ok(())
        };
        if let Some(tree) = &tree {
            let indices = tree.query(&polygons, &subject.bbox, &mut budget)?;
            visit(&indices, &mut budget)?;
        } else if let Some(grid) = &grid {
            if let Some((x0, y0, x1, y1)) = grid.cells_for(&subject.bbox) {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        visit(&grid.cells[y * grid.nx + x], &mut budget)?;
                    }
                }
                visit(&grid.large, &mut budget)?;
            }
        }
        budget.candidates = candidates.len();
        budget.spend(candidates.len() * bits_len(candidates.len()))?;
        candidates.sort_by(|&i, &j| {
            if polygons[i].area != polygons[j].area {
                return polygons[j]
                    .area
                    .partial_cmp(&polygons[i].area)
                    .unwrap_or(std::cmp::Ordering::Equal);
            }
            if faces[i].order == faces[j].order {
                return j.cmp(&i);
            }
            faces[j].order.cmp(&faces[i].order)
        });
        let mut indexed = if candidates.len() > 64 {
            Some(FragmentSet::new(
                &subject.points,
                &mut budget,
                limits.fragments - output_fragments,
                limits.vertices - output_vertices,
            )?)
        } else {
            None
        };
        for &other in &candidates {
            let occluder = &polygons[other];
            budget.spend(occluder.points.len() + 4)?;
            let subject_plane = subject.plane;
            let occluder_plane = occluder.plane;
            let difference = move |p: SvgPoint| occluder_plane.at(p) - subject_plane.at(p);
            let (mut lo, mut hi, mut depth_magnitude) = (f64::INFINITY, f64::NEG_INFINITY, 1.0f64);
            let b = SvgBox {
                min_x: go_max(subject.bbox.min_x, occluder.bbox.min_x),
                min_y: go_max(subject.bbox.min_y, occluder.bbox.min_y),
                max_x: go_min(subject.bbox.max_x, occluder.bbox.max_x),
                max_y: go_min(subject.bbox.max_y, occluder.bbox.max_y),
            };
            for (x, y) in [
                (b.min_x, b.min_y),
                (b.max_x, b.min_y),
                (b.max_x, b.max_y),
                (b.min_x, b.max_y),
            ] {
                let p = SvgPoint { x, y, z: 0.0 };
                let d = difference(p);
                lo = go_min(lo, d);
                hi = go_max(hi, d);
                depth_magnitude = go_max(
                    depth_magnitude,
                    go_max(subject_plane.at(p).abs(), occluder_plane.at(p).abs()),
                );
            }
            let epsilon = go_max(1e-9, depth_magnitude * 2e-14);
            let mut cut = occluder.points.clone();
            if lo >= -epsilon && hi <= epsilon {
                if faces[other].order < faces[index].order
                    || (faces[other].order == faces[index].order && other < index)
                {
                    continue;
                }
            } else if hi <= 0.0 {
                continue;
            } else if lo < 0.0 {
                let (inside, _) = split_polygon(&cut, &difference);
                match inside.resolve(&cut) {
                    Some(points) => cut = points,
                    None => continue,
                }
            }
            if let Some(set) = indexed.as_mut() {
                set.subtract(&cut, &mut budget)?;
                budget.fragments = set.fragments.len();
                if budget.fragments == 0 {
                    break;
                }
                continue;
            }
            let cut_box = polygon_box(&cut);
            let mut next = Vec::new();
            let mut vertices = 0;
            for fragment in &fragments {
                budget.spend(fragment.len() + 1)?;
                let pieces = if polygon_box(fragment).overlaps(&cut_box) {
                    subtract_polygon(fragment, &cut, &mut budget)?.0
                } else {
                    vec![fragment.clone()]
                };
                vertices += pieces.iter().map(Vec::len).sum::<usize>();
                next.extend(pieces);
                if next.len() > limits.fragments - output_fragments
                    || vertices > limits.vertices - output_vertices
                {
                    return Err("isometric SVG visibility exceeds fragment limit".into());
                }
            }
            fragments = next;
            budget.fragments = fragments.len();
            if fragments.is_empty() {
                break;
            }
        }
        if let Some(set) = indexed {
            fragments = set.polygons(&mut budget)?;
        }
        output_fragments += fragments.len();
        output_vertices += fragments.iter().map(Vec::len).sum::<usize>();
        if output_fragments > limits.fragments || output_vertices > limits.vertices {
            return Err("isometric SVG visibility exceeds fragment limit".into());
        }
        output[index] = fragments;
    }
    Ok(output)
}

#[cfg(test)]
#[path = "svg_precision_tests.rs"]
mod source_contracts;
