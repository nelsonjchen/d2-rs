// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Upstream 64288a936 ovg_crossing_index.go: fixed grid interval index.
use super::Point;
use super::routed_edge_set::Work;
use super::static_edge_index::{Axes, StaticEdges};

#[derive(Clone, Copy, Debug)]
pub(super) struct Interval {
    pub(super) edge: usize,
    start: f64,
    end: f64,
    max_end: f64,
}
#[derive(Debug)]
struct Line {
    coordinate: f64,
    intervals: Vec<Interval>,
}
#[derive(Debug, Default)]
pub(super) struct Lines(Vec<Line>);
fn add(work: Option<&dyn Work>, amount: u64) -> Result<(), ()> {
    work.map_or(Ok(()), |w| w.add(amount))
}
fn step(work: Option<&dyn Work>) -> Result<(), ()> {
    work.map_or(Ok(()), Work::step)
}
fn bit_len(n: usize) -> u64 {
    (usize::BITS - n.leading_zeros()) as u64
}
fn sort_work(work: Option<&dyn Work>, n: usize) -> Result<(), ()> {
    if n < 2 {
        step(work)
    } else {
        add(work, n as u64 * bit_len(n - 1))
    }
}
impl Lines {
    fn new(
        axes: &Axes,
        edges: &StaticEdges,
        vertical: bool,
        work: Option<&dyn Work>,
    ) -> Result<Self, ()> {
        sort_work(work, axes.buckets.len())?;
        let mut lines = Vec::with_capacity(axes.buckets.len());
        for bucket in axes.buckets.values() {
            sort_work(work, bucket.edges.len())?;
            let mut intervals = Vec::with_capacity(bucket.edges.len());
            for &edge in &bucket.edges {
                step(work)?;
                let [a, b] = edges.edge_points(edge);
                let (a, b) = if vertical { (a.y, b.y) } else { (a.x, b.x) };
                intervals.push(Interval {
                    edge,
                    start: a.min(b),
                    end: a.max(b),
                    max_end: 0.0,
                });
            }
            intervals.sort_by(|a, b| a.start.total_cmp(&b.start));
            let mut max_end = f64::NEG_INFINITY;
            for i in &mut intervals {
                step(work)?;
                max_end = max_end.max(i.end);
                i.max_end = max_end;
            }
            lines.push(Line {
                coordinate: bucket.axis,
                intervals,
            });
        }
        lines.sort_by(|a, b| a.coordinate.total_cmp(&b.coordinate));
        work.map_or(Ok(()), Work::check)?;
        Ok(Self(lines))
    }
    pub(super) fn visit(
        &self,
        first: f64,
        last: f64,
        start: f64,
        end: f64,
        work: Option<&dyn Work>,
        mut found: impl FnMut(f64, Interval),
    ) -> Result<(), ()> {
        add(work, bit_len(self.0.len()))?;
        let i = self.0.partition_point(|l| l.coordinate < first);
        for line in &self.0[i..] {
            step(work)?;
            if line.coordinate > last {
                break;
            }
            add(work, bit_len(line.intervals.len()))?;
            let j = line.intervals.partition_point(|i| i.max_end < start);
            for &interval in &line.intervals[j..] {
                step(work)?;
                if interval.start > end {
                    break;
                }
                if interval.end >= start {
                    found(line.coordinate, interval);
                }
            }
        }
        work.map_or(Ok(()), Work::check)
    }
}
#[derive(Debug, Default)]
pub(super) struct CrossingIndex {
    pub(super) horizontal: Lines,
    pub(super) vertical: Lines,
    pub(super) crossed: super::static_edge_index::EdgeFlags,
}
impl CrossingIndex {
    pub(super) fn new(edges: &StaticEdges, work: Option<&dyn Work>) -> Result<Self, ()> {
        Ok(Self {
            horizontal: Lines::new(&edges.horizontal, edges, false, work)?,
            vertical: Lines::new(&edges.vertical, edges, true, work)?,
            crossed: Default::default(),
        })
    }
    pub(super) fn add(
        &mut self,
        segment: [Point; 2],
        edges: &StaticEdges,
        work: Option<&dyn Work>,
    ) -> Result<(), ()> {
        let [a, b] = segment;
        let (axis, from, to, parallel, perpendicular) = if a.x == b.x {
            (a.x, a.y, b.y, &self.vertical, &self.horizontal)
        } else if a.y == b.y {
            (a.y, a.x, b.x, &self.horizontal, &self.vertical)
        } else {
            return Ok(());
        };
        let (start, end) = (from.min(to), from.max(to));
        let shares = |id| {
            let [c, d] = edges.edge_points(id);
            a == c || a == d || b == c || b == d
        };
        parallel.visit(axis, axis, start, end, work, |_, i| {
            if start < i.end && i.start < end && !shares(i.edge) {
                self.crossed.insert(i.edge);
            }
        })?;
        perpendicular.visit(start, end, axis, axis, work, |_, i| {
            if !shares(i.edge) {
                self.crossed.insert(i.edge);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::routed_edge_set::{Edge, EdgeSet};
    use super::super::static_edge_index::StaticEdgeIndex;
    use super::*;
    #[test]
    fn crossing_index_matches_route_segments() {
        // Upstream ovg_crossing_index_test.go TestCrossingIndexMatchesRouteSegments.
        let p = |x, y| Point { x, y };
        let mut edges = Vec::new();
        for i in 0..12 {
            let axis = i as f64 + 0.5;
            edges.extend([
                [p(0., axis), p(12., axis)],
                [p(axis, 12.), p(axis, 0.)],
                [p(axis, axis), p(axis, axis + 3.)],
            ]);
        }
        let grid = StaticEdgeIndex::from_segments(edges.clone());
        let mut index = CrossingIndex::new(&grid, None).unwrap();
        let mut set = EdgeSet::default();
        for segment in [
            [p(2.5, 6.5), p(8.5, 6.5)],
            [p(5.5, 12.), p(5.5, 4.5)],
            [p(2.5, 3.5), p(2.5, 5.5)],
            [p(12., 8.5), p(6.5, 8.5)],
            [p(0., 0.), p(12., 12.)],
        ] {
            set.add(Edge::fixture(segment));
            index.add(segment, &grid, None).unwrap();
            for (i, &edge) in edges.iter().enumerate() {
                assert_eq!(
                    index.crossed.contains(&i),
                    set.intersects_with::<dyn Work>(&Edge::fixture(edge), None)
                        .unwrap(),
                    "segment {segment:?}, edge {edge:?}"
                );
            }
        }
    }
}
