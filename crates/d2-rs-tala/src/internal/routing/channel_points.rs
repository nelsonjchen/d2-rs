// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Identity-preserving channelPointProposal/channelWireLength translation.
//! IDs refer to actual shared point slots, never coordinate-based deduplication.
use crate::Point;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PointId(pub usize);

pub(super) struct Proposal {
    /// Slot lookup only; this is not a claim about upstream Go map traversal.
    values: Vec<Option<Point>>,
}
impl Proposal {
    pub(super) fn updates(&self) -> impl Iterator<Item = (PointId, Point)> + '_ {
        self.values
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.map(|point| (PointId(i), point)))
    }

    pub(super) fn point(&self, id: PointId, original: &[Point]) -> Point {
        self.values[id.0].unwrap_or(original[id.0])
    }
}

/// Each group retains segment order, and each segment retains endpoint order.
/// None is an incompatible proposal to the same shared point.
pub(super) fn propose<E>(
    points: &[Point],
    groups: &[Vec<[PointId; 2]>],
    positions: &[f64],
    horizontal: bool,
    mut reserve: impl FnMut(u64) -> Result<(), E>,
) -> Result<Option<Proposal>, E> {
    let mut proposal = Proposal {
        values: vec![None; points.len()],
    };
    for (i, group) in groups.iter().enumerate() {
        for segment in group {
            for &id in segment {
                reserve(1)?;
                let mut candidate = points[id.0];
                if horizontal {
                    candidate.x = positions[i];
                } else {
                    candidate.y = positions[i];
                }
                if let Some(old) = proposal.values[id.0]
                    && old != candidate
                {
                    return Ok(None);
                }
                proposal.values[id.0] = Some(candidate);
            }
        }
    }
    Ok(Some(proposal))
}

// Translation of Go 1.27 src/math/hypot.go, used on the pinned ARM64 oracle.
// The ARM64 Go implementation lowers Sqrt to the platform FSQRTD instruction;
// keeping this helper explicit makes the operation order in Hypot visible.
// Copyright 2010 The Go Authors. All rights reserved.
// BSD terms: see THIRD_PARTY_NOTICES.md.
pub(in crate::internal) fn go_sqrt(x: f64) -> f64 {
    x.sqrt()
}

pub(in crate::internal) fn go_hypot(mut p: f64, mut q: f64) -> f64 {
    p = p.abs();
    q = q.abs();
    if p == f64::INFINITY || q == f64::INFINITY {
        return f64::INFINITY;
    }
    if p.is_nan() || q.is_nan() {
        return f64::from_bits(0x7ff8_0000_0000_0001);
    }
    if p < q {
        std::mem::swap(&mut p, &mut q);
    }
    if p == 0.0 {
        return 0.0;
    }
    q /= p;
    // Go's ARM64 compiler fuses `1+q*q` in math.hypot into an FMA before
    // calling FSQRTD. Preserve that intermediate exactly; a separate multiply
    // and add can be one ULP lower.
    p * go_sqrt(d2_rs_core::fused_mul_add(q, q, 1.0))
}

pub(super) fn wire_length<E>(
    points: &[Point],
    edges: &[Vec<PointId>],
    proposal: Option<&Proposal>,
    mut reserve: impl FnMut(u64) -> Result<(), E>,
) -> Result<f64, E> {
    let mut length = 0.0;
    for edge in edges {
        for pair in edge.windows(2) {
            reserve(1)?;
            let get = |id: PointId| proposal.map_or(points[id.0], |p| p.point(id, points));
            let a = get(pair[0]);
            let b = get(pair[1]);
            length += go_hypot(b.x - a.x, b.y - a.y);
        }
    }
    Ok(length)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn allow(_: u64) -> Result<(), ()> {
        Ok(())
    }
    fn point(x: f64, y: f64) -> Point {
        Point { x, y }
    }
    #[test]
    fn shared_identity_is_not_coordinate_equality() {
        let points = vec![point(0.0, 0.0), point(0.0, 0.0), point(10.0, 0.0)];
        let groups = vec![vec![[PointId(0), PointId(2)]]];
        let proposal = propose(&points, &groups, &[5.0], false, allow)
            .unwrap()
            .unwrap();
        assert_eq!(proposal.point(PointId(0), &points), point(0.0, 5.0));
        assert_eq!(proposal.point(PointId(1), &points), point(0.0, 0.0));
        let edges = vec![vec![PointId(0), PointId(2)], vec![PointId(1), PointId(0)]];
        assert_eq!(
            wire_length(&points, &edges, Some(&proposal), allow).unwrap(),
            15.0
        );
        assert_eq!(points[0], point(0.0, 0.0));
    }
    #[test]
    fn conflicting_shared_point_rejects_without_mutation() {
        let points = vec![point(0.0, 0.0), point(10.0, 0.0), point(20.0, 0.0)];
        let groups = vec![
            vec![[PointId(0), PointId(1)]],
            vec![[PointId(1), PointId(2)]],
        ];
        assert!(
            propose(&points, &groups, &[5.0, 6.0], false, allow)
                .unwrap()
                .is_none()
        );
        assert!(
            propose(&points, &groups, &[5.0, 5.0], false, allow)
                .unwrap()
                .is_some()
        );
        assert_eq!(points[1], point(10.0, 0.0));
    }
    #[test]
    fn cancelled_proposal_does_not_expose_partial_updates() {
        let points = vec![point(0.0, 0.0), point(10.0, 0.0)];
        let original = points.clone();
        let mut calls = 0;
        let result = propose(
            &points,
            &[vec![[PointId(0), PointId(1)]]],
            &[5.0],
            false,
            |_| {
                calls += 1;
                if calls == 2 { Err("cancelled") } else { Ok(()) }
            },
        );
        assert!(matches!(result, Err("cancelled")));
        assert_eq!(points, original);
    }
    #[test]
    fn wire_length_preserves_euclidean_distance_and_work_order() {
        let points = vec![point(0.0, 0.0), point(3.0, 4.0), point(3.0, 16.0)];
        let edges = vec![vec![PointId(0), PointId(1)], vec![PointId(1), PointId(2)]];
        let mut charges = vec![];
        assert_eq!(
            wire_length(&points, &edges, None, |u| {
                charges.push(u);
                Ok::<_, ()>(())
            })
            .unwrap(),
            17.0
        );
        assert_eq!(charges, vec![1, 1]);
        assert_eq!(go_hypot(-0.0, 0.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(go_hypot(f64::NAN, f64::INFINITY), f64::INFINITY);
        assert!(go_hypot(f64::NAN, 1.0).is_nan());
    }

    #[test]
    fn go_hypot_matches_go_arm64_result() {
        assert_eq!(
            go_sqrt((142.0_f64 / 283.5).mul_add(142.0_f64 / 283.5, 1.0)).to_bits(),
            0x3ff1e5156cabb585
        );
        assert_eq!(go_hypot(-283.5, -142.0).to_bits(), 0x4073d13139d82785);
        assert_eq!(go_hypot(-212.5, -142.0).to_bits(), 0x406ff281277ca342);
    }
}
