// SPDX-License-Identifier: MPL-2.0
//! Exact-query broad phase from d2elklayout/spatial_index.go.
use d2_rs_core::{EdgeId, Graph, LayoutResult, NodeId, Point};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
pub(super) struct Bounds {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}
impl Bounds {
    pub fn segment(a: Point, b: Point) -> Self {
        Self {
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        }
    }
    pub fn expanded(self, d: f64) -> Self {
        Self {
            x0: self.x0 - d,
            y0: self.y0 - d,
            x1: self.x1 + d,
            y1: self.y1 + d,
        }
    }
    fn overlaps(self, b: Self) -> bool {
        self.x0 <= b.x1 && b.x0 <= self.x1 && self.y0 <= b.y1 && b.y0 <= self.y1
    }
    fn cells(self) -> Option<Vec<(i32, i32)>> {
        let v = [self.x0, self.y0, self.x1, self.y1].map(|x| (x / 128.0).floor());
        if v.iter().any(|x| !x.is_finite() || x.abs() > 2147483646.0) {
            return None;
        }
        let w = v[2] - v[0] + 1.0;
        let h = v[3] - v[1] + 1.0;
        if w <= 0.0 || h <= 0.0 || w > 256.0 || h > 256.0 || w * h > 256.0 {
            return None;
        }
        let mut cells = Vec::with_capacity((w * h) as usize);
        for y in v[1] as i32..=v[3] as i32 {
            for x in v[0] as i32..=v[2] as i32 {
                cells.push((x, y));
            }
        }
        Some(cells)
    }
}
struct Entry {
    bounds: Bounds,
    cells: Option<Vec<(i32, i32)>>,
}
struct SpatialIndex<K: Copy + Ord> {
    entries: BTreeMap<K, Entry>,
    cells: BTreeMap<(i32, i32), BTreeSet<K>>,
    large: BTreeSet<K>,
}
impl<K: Copy + Ord> Default for SpatialIndex<K> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            cells: BTreeMap::new(),
            large: BTreeSet::new(),
        }
    }
}
impl<K: Copy + Ord> SpatialIndex<K> {
    fn remove(&mut self, key: K) {
        let Some(entry) = self.entries.remove(&key) else {
            return;
        };
        if let Some(cells) = entry.cells {
            for cell in cells {
                if let Some(items) = self.cells.get_mut(&cell) {
                    items.remove(&key);
                    if items.is_empty() {
                        self.cells.remove(&cell);
                    }
                }
            }
        } else {
            self.large.remove(&key);
        }
    }
    fn add(&mut self, key: K, bounds: Bounds) {
        self.remove(key);
        let cells = bounds.cells();
        if let Some(cells) = &cells {
            for &cell in cells {
                self.cells.entry(cell).or_default().insert(key);
            }
        } else {
            self.large.insert(key);
        }
        self.entries.insert(key, Entry { bounds, cells });
    }
    fn query(&self, bounds: Bounds) -> Vec<K> {
        let Some(cells) = bounds.cells() else {
            return self
                .entries
                .iter()
                .filter_map(|(&k, e)| e.bounds.overlaps(bounds).then_some(k))
                .collect();
        };
        let mut seen = self.large.clone();
        for cell in cells {
            if let Some(items) = self.cells.get(&cell) {
                seen.extend(items);
            }
        }
        seen.into_iter()
            .filter(|k| self.entries[k].bounds.overlaps(bounds))
            .collect()
    }
}
pub(super) struct CollisionIndex {
    objects: SpatialIndex<NodeId>,
    segments: SpatialIndex<(EdgeId, usize)>,
    lengths: BTreeMap<EdgeId, usize>,
}
impl CollisionIndex {
    pub fn new(graph: &Graph, result: &LayoutResult) -> Self {
        let mut index = Self {
            objects: SpatialIndex::default(),
            segments: SpatialIndex::default(),
            lengths: BTreeMap::new(),
        };
        for (id, _) in graph.nodes() {
            if let Some(b) = result.boxes.get(&id) {
                index.objects.add(
                    id,
                    Bounds {
                        x0: b.origin.x,
                        y0: b.origin.y,
                        x1: b.origin.x + b.size.width,
                        y1: b.origin.y + b.size.height,
                    }
                    .expanded(39.0),
                );
            }
        }
        for (id, _) in graph.edges() {
            index.update_edge(id, result);
        }
        index
    }
    pub fn update_edge(&mut self, id: EdgeId, result: &LayoutResult) {
        for i in 0..self.lengths.remove(&id).unwrap_or(0) {
            self.segments.remove((id, i));
        }
        if let Some(route) = result.routes.get(&id) {
            self.lengths.insert(id, route.len().saturating_sub(1));
            for (i, p) in route.windows(2).enumerate() {
                self.segments.add((id, i), Bounds::segment(p[0], p[1]));
            }
        }
    }
    pub fn objects(&self, a: Point, b: Point) -> Vec<NodeId> {
        self.objects.query(Bounds::segment(a, b))
    }
    pub fn segments(&self, a: Point, b: Point) -> Vec<(EdgeId, usize)> {
        self.segments.query(Bounds::segment(a, b).expanded(20.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_and_wasm32_coordinates_use_exact_bounded_fallback() {
        let huge = (1u64 << 31) as f64 * 128.0;
        for x in [huge, -huge] {
            let bounds = Bounds::segment(Point { x, y: 0.0 }, Point { x, y: 1.0 });
            assert!(bounds.cells().is_none());
            let mut index = SpatialIndex::default();
            index.add(0usize, bounds);
            assert_eq!(index.query(bounds), vec![0]);
            index.remove(0);
            assert!(index.query(bounds).is_empty());
        }
        let bounds = Bounds::segment(
            Point {
                x: -100000.0,
                y: 20.0,
            },
            Point {
                x: 100000.0,
                y: 20.0,
            },
        );
        assert!(bounds.cells().is_none());
        let mut index = SpatialIndex::default();
        index.add(1usize, bounds);
        let small = Bounds::segment(Point { x: 0.0, y: 20.0 }, Point { x: 1.0, y: 20.0 });
        assert_eq!(index.query(small), vec![1]);
        index.add(
            1,
            Bounds::segment(Point { x: 0.0, y: 21.0 }, Point { x: 1.0, y: 21.0 }),
        );
        assert!(index.query(small).is_empty());
    }
}
