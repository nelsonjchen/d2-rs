// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Channel grouping and bounds from D2 v0.9.0 buildChannelProblem.
//! Input segments must be in upstream inventory order and canonically directed.
//! The graph adapter must resolve pointer identity and locked edges beforehand.
use super::channel_solver::{Arc, Group, Problem, maximum, minimum};
use crate::Point;

const SPACING: f64 = 40.0;

#[derive(Clone, Copy)]
pub(super) struct Segment {
    pub start: Point,
    pub end: Point,
    pub owner: usize,
    pub fixed: bool,
}

impl Segment {
    fn coordinate(self, horizontal: bool) -> f64 {
        if horizontal {
            self.start.x
        } else {
            self.start.y
        }
    }
    fn overlaps(self, other: Self, horizontal: bool, buffer: f64) -> bool {
        let (a, b, c, d) = if horizontal {
            (self.start.y, self.end.y, other.start.y, other.end.y)
        } else {
            (self.start.x, self.end.x, other.start.x, other.end.x)
        };
        !(minimum(a, b) - maximum(c, d) >= buffer || minimum(c, d) - maximum(a, b) >= buffer)
    }
}

struct GeometricGroup {
    segments: Vec<usize>,
    position: f64,
    lower: f64,
    upper: f64,
    fixed: bool,
    clearance: Option<(f64, f64)>,
}

pub(super) struct Channels {
    pub problem: Problem,
    /// Corresponding inventory indices; point identities remain in the adapter.
    pub segments: Vec<Vec<usize>>,
    pub positions: Vec<f64>,
}

fn root(parent: &mut [usize], mut index: usize) -> usize {
    while parent[index] != index {
        parent[index] = parent[parent[index]];
        index = parent[index];
    }
    index
}

/// routeSegmentBounds, including inclusive movement bands and guard ordering.
fn bounds<E>(
    segment: Segment,
    walls: &[Segment],
    reserve: &mut impl FnMut(u64) -> Result<(), E>,
) -> Result<(f64, f64), E> {
    let mut lower = f64::NEG_INFINITY;
    let mut upper = f64::INFINITY;
    if segment.start == segment.end {
        reserve(1)?;
        return Ok((lower, upper));
    }
    let horizontal = segment.start.x == segment.end.x;
    for wall in walls {
        reserve(1)?;
        let (start, end, other_start, other_end, coordinate, other_coordinate) = if horizontal {
            (
                segment.start.y,
                segment.end.y,
                wall.start.y,
                wall.end.y,
                segment.start.x,
                wall.start.x,
            )
        } else {
            (
                segment.start.x,
                segment.end.x,
                wall.start.x,
                wall.end.x,
                segment.start.y,
                wall.start.y,
            )
        };
        if other_end < start - SPACING || other_start > end + SPACING {
            continue;
        }
        if other_coordinate <= coordinate {
            lower = maximum(lower, other_coordinate);
        }
        if other_coordinate > coordinate {
            upper = minimum(upper, other_coordinate);
        }
    }
    Ok((lower, upper))
}

/// Stable bottom-up merge sort with upstream work charges. Sort group indices
/// rather than owning groups; this retains the pointer-sort semantics.
fn sort_groups<E>(
    order: &mut [usize],
    groups: &[GeometricGroup],
    reserve: &mut impl FnMut(u64) -> Result<(), E>,
) -> Result<(), E> {
    if order.len() < 2 {
        return reserve(1);
    }
    let mut temporary = vec![0; order.len()];
    let mut width = 1;
    while width < order.len() {
        for start in (0..order.len()).step_by(2 * width) {
            let middle = (start + width).min(order.len());
            let end = (start + 2 * width).min(order.len());
            let (mut left, mut right, mut output) = (start, middle, start);
            while left < middle && right < end {
                reserve(1)?;
                if groups[order[right]].position < groups[order[left]].position {
                    temporary[output] = order[right];
                    right += 1;
                } else {
                    temporary[output] = order[left];
                    left += 1;
                }
                output += 1;
            }
            while left < middle {
                reserve(1)?;
                temporary[output] = order[left];
                left += 1;
                output += 1;
            }
            while right < end {
                reserve(1)?;
                temporary[output] = order[right];
                right += 1;
                output += 1;
            }
        }
        reserve(order.len() as u64)?;
        order.copy_from_slice(&temporary);
        if width > order.len() / 2 {
            break;
        }
        width *= 2;
    }
    Ok(())
}

pub(super) fn build<E>(
    segments: &[Segment],
    node_walls: &[Segment],
    horizontal: bool,
    mut reserve: impl FnMut(u64) -> Result<(), E>,
) -> Result<Channels, E> {
    let mut parent: Vec<_> = (0..segments.len()).collect();
    for (i, s) in segments.iter().enumerate() {
        for (j, other) in segments.iter().enumerate().take(i) {
            reserve(1)?;
            if s.coordinate(horizontal) == other.coordinate(horizontal)
                && s.overlaps(*other, horizontal, SPACING)
            {
                let a = root(&mut parent, i);
                let b = root(&mut parent, j);
                parent[a] = b;
            }
        }
    }
    let mut by_root = vec![None; segments.len()];
    let mut groups: Vec<GeometricGroup> = Vec::new();
    for (i, s) in segments.iter().enumerate() {
        reserve(1)?;
        let id = root(&mut parent, i);
        let group_index = *by_root[id].get_or_insert_with(|| {
            groups.push(GeometricGroup {
                segments: Vec::new(),
                position: s.coordinate(horizontal),
                lower: 0.0,
                upper: 0.0,
                fixed: false,
                clearance: None,
            });
            groups.len() - 1
        });
        groups[group_index].segments.push(i);
        groups[group_index].fixed |= s.fixed;
    }
    for group in &mut groups {
        if group.fixed {
            continue;
        }
        let mut clearance = (f64::NEG_INFINITY, f64::INFINITY);
        for &index in &group.segments {
            let (lower, upper) = bounds(segments[index], node_walls, &mut reserve)?;
            if lower != f64::NEG_INFINITY {
                clearance.0 = maximum(
                    clearance.0,
                    lower + minimum(group.position - lower, SPACING),
                );
            }
            if upper != f64::INFINITY {
                clearance.1 = minimum(
                    clearance.1,
                    upper - minimum(upper - group.position, SPACING),
                );
            }
        }
        group.clearance = Some(clearance);
    }
    let mut walls = node_walls.to_vec();
    for group in &groups {
        if group.fixed {
            walls.extend(group.segments.iter().map(|&i| segments[i]));
        }
    }
    for _ in 0..2 {
        for group in &mut groups {
            if group.fixed {
                continue;
            }
            group.lower = f64::NEG_INFINITY;
            group.upper = f64::INFINITY;
            for &index in &group.segments {
                let (lo, hi) = bounds(segments[index], &walls, &mut reserve)?;
                group.lower = maximum(group.lower, lo);
                group.upper = minimum(group.upper, hi);
            }
            if group.lower.is_infinite()
                || group.upper.is_infinite()
                || group.lower > group.position
                || group.upper < group.position
                || group.lower >= group.upper
            {
                group.fixed = true;
                walls.extend(group.segments.iter().map(|&i| segments[i]));
            }
        }
    }
    let mut order: Vec<_> = groups
        .iter()
        .enumerate()
        .filter_map(|(i, g)| (!g.fixed).then_some(i))
        .collect();
    sort_groups(&mut order, &groups, &mut reserve)?;
    let mut arcs = Vec::new();
    for (i, &a_index) in order.iter().enumerate() {
        let a = &groups[a_index];
        for (j, &b_index) in order.iter().enumerate().skip(i + 1) {
            let b = &groups[b_index];
            let owner = segments[a.segments[0]].owner;
            let same_owner = a
                .segments
                .iter()
                .chain(&b.segments)
                .all(|&s| segments[s].owner == owner);
            let buffer = if same_owner { 0.0 } else { SPACING };
            let mut linked = false;
            for &s in &a.segments {
                for &t in &b.segments {
                    reserve(1)?;
                    linked |= segments[s].overlaps(segments[t], horizontal, buffer);
                }
            }
            if linked {
                arcs.push(Arc {
                    from: i,
                    to: j,
                    separate: !same_owner,
                });
            }
        }
    }
    Ok(Channels {
        problem: Problem {
            groups: order
                .iter()
                .map(|&i| Group {
                    lower: groups[i].lower,
                    upper: groups[i].upper,
                    node_clearance: groups[i].clearance,
                })
                .collect(),
            arcs,
        },
        positions: order.iter().map(|&i| groups[i].position).collect(),
        segments: order.iter().map(|&i| groups[i].segments.clone()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn segment(y: f64, owner: usize, fixed: bool) -> Segment {
        Segment {
            start: Point { x: 0.0, y },
            end: Point { x: 100.0, y },
            owner,
            fixed,
        }
    }
    fn allow(_: u64) -> Result<(), ()> {
        Ok(())
    }
    #[test]
    fn unbounded_outer_channel_is_fixed() {
        let result = build(
            &[segment(70.0, 0, false)],
            &[segment(0.0, 1, true)],
            false,
            allow,
        )
        .unwrap();
        assert!(result.problem.groups.is_empty());
    }
    #[test]
    fn shared_trunk_uses_one_group_and_clearance() {
        let segments = [
            segment(60.0, 0, false),
            segment(60.0, 1, false),
            segment(100.0, 2, false),
        ];
        let walls = [segment(0.0, 3, true), segment(200.0, 4, true)];
        let result = build(&segments, &walls, false, allow).unwrap();
        assert_eq!(result.positions, vec![60.0, 100.0]);
        assert_eq!(result.segments, vec![vec![0, 1], vec![2]]);
        assert_eq!(result.problem.groups[0].node_clearance, Some((40.0, 160.0)));
        assert_eq!(result.problem.arcs.len(), 1);
        assert!(result.problem.arcs[0].separate);
        assert!(result.problem.solve_gap(40.0, allow).unwrap().is_some());
    }
    #[test]
    fn fixed_member_locks_shared_group() {
        let result = build(
            &[segment(60.0, 0, false), segment(60.0, 1, true)],
            &[segment(0.0, 2, true), segment(200.0, 3, true)],
            false,
            allow,
        )
        .unwrap();
        assert!(result.problem.groups.is_empty());
    }
    #[test]
    fn movement_band_is_inclusive_but_group_overlap_is_strict() {
        let a = segment(50.0, 0, false);
        let mut b = segment(0.0, 1, true);
        b.start.x = 140.0;
        b.end.x = 200.0;
        assert!(!a.overlaps(b, false, 40.0));
        assert_eq!(bounds(a, &[b], &mut allow).unwrap().0, 0.0);
    }
    #[test]
    fn cancellation_interrupts_discovery() {
        let mut charges = 0;
        let result = build(
            &[segment(60.0, 0, false), segment(100.0, 1, false)],
            &[],
            false,
            |_| {
                charges += 1;
                Err("cancelled")
            },
        );
        assert!(matches!(result, Err("cancelled")));
        assert_eq!(charges, 1);
    }
}
