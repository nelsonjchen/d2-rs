// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! D2 v0.9.0 nudge_channels.go: solveChannelGap and channelGap.
//! Groups and arcs must retain the order produced by buildChannelProblem.

#[derive(Clone)]
pub(super) struct Group {
    pub lower: f64,
    pub upper: f64,
    pub node_clearance: Option<(f64, f64)>,
}

pub(super) struct Arc {
    pub from: usize,
    pub to: usize,
    pub separate: bool,
}

pub(super) struct Problem {
    pub groups: Vec<Group>,
    pub arcs: Vec<Arc>,
}

// Go math.Min/Max propagate NaN and specify the sign of equal zero values.
// Rust f64::min/max discard NaNs, so they are not direct translations.
pub(in crate::internal) fn minimum(a: f64, b: f64) -> f64 {
    if a == f64::NEG_INFINITY || b == f64::NEG_INFINITY {
        f64::NEG_INFINITY
    } else if a.is_nan() || b.is_nan() {
        f64::from_bits(0x7ff8_0000_0000_0001)
    } else if a == b && a == 0.0 {
        f64::from_bits(a.to_bits() | b.to_bits())
    } else if a < b {
        a
    } else {
        b
    }
}
pub(in crate::internal) fn maximum(a: f64, b: f64) -> f64 {
    if a == f64::INFINITY || b == f64::INFINITY {
        f64::INFINITY
    } else if a.is_nan() || b.is_nan() {
        f64::from_bits(0x7ff8_0000_0000_0001)
    } else if a == b && a == 0.0 {
        f64::from_bits(a.to_bits() & b.to_bits())
    } else if a > b {
        a
    } else {
        b
    }
}

impl Problem {
    /// Charge allocation work before allocating and each traversal step before
    /// visiting the arc, in the same order as upstream. None means infeasible.
    pub(super) fn solve_gap<E>(
        &self,
        gap: f64,
        mut reserve: impl FnMut(u64) -> Result<(), E>,
    ) -> Result<Option<(Vec<f64>, Vec<f64>)>, E> {
        reserve(self.groups.len() as u64 * 2)?;
        let mut lo = Vec::with_capacity(self.groups.len());
        let mut hi = Vec::with_capacity(self.groups.len());
        for group in &self.groups {
            let mut lower = group.lower + gap;
            let mut upper = group.upper - gap;
            if let Some((start, end)) = group.node_clearance {
                lower = maximum(lower, start);
                upper = minimum(upper, end);
            }
            lo.push(lower);
            hi.push(upper);
        }
        for arc in &self.arcs {
            reserve(1)?;
            let separation = if arc.separate { gap } else { 0.0 };
            lo[arc.to] = maximum(lo[arc.to], lo[arc.from] + separation);
        }
        for arc in self.arcs.iter().rev() {
            reserve(1)?;
            let separation = if arc.separate { gap } else { 0.0 };
            hi[arc.from] = minimum(hi[arc.from], hi[arc.to] - separation);
        }
        if lo.iter().zip(&hi).any(|(lo, hi)| lo > hi) {
            return Ok(None);
        }
        Ok(Some((lo, hi)))
    }

    pub(super) fn gap(&self, positions: &[f64]) -> f64 {
        let mut gap = f64::INFINITY;
        for (i, group) in self.groups.iter().enumerate() {
            gap = minimum(
                gap,
                minimum(positions[i] - group.lower, group.upper - positions[i]),
            );
        }
        for arc in &self.arcs {
            if arc.separate {
                gap = minimum(gap, positions[arc.to] - positions[arc.from]);
            }
        }
        gap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn group(lower: f64, upper: f64) -> Group {
        Group {
            lower,
            upper,
            node_clearance: None,
        }
    }
    fn allow(_: u64) -> Result<(), ()> {
        Ok(())
    }

    #[test]
    fn upstream_coupled_different_intervals() {
        let p = Problem {
            groups: vec![group(0.0, 120.0), group(20.0, 180.0)],
            arcs: vec![Arc {
                from: 0,
                to: 1,
                separate: true,
            }],
        };
        let (lo, hi) = p.solve_gap(60.0, allow).unwrap().unwrap();
        assert_eq!(lo, vec![60.0, 120.0]);
        assert_eq!(lo, hi);
        assert!(p.solve_gap(61.0, allow).unwrap().is_none());
        let (lo, hi) = p.solve_gap(20.0, allow).unwrap().unwrap();
        let mid: Vec<_> = lo.iter().zip(hi).map(|(a, b)| (a + b) / 2.0).collect();
        assert!(p.gap(&mid) >= 20.0);
    }
    #[test]
    fn upstream_same_path_legs_may_meet() {
        let p = Problem {
            groups: vec![group(0.0, 100.0), group(0.0, 100.0)],
            arcs: vec![Arc {
                from: 0,
                to: 1,
                separate: false,
            }],
        };
        assert_eq!(
            p.solve_gap(50.0, allow).unwrap(),
            Some((vec![50.0, 50.0], vec![50.0, 50.0]))
        );
    }
    #[test]
    fn upstream_absolute_node_clearance() {
        let p = Problem {
            groups: vec![Group {
                lower: 0.0,
                upper: 100.0,
                node_clearance: Some((10.0, 30.0)),
            }],
            arcs: vec![],
        };
        assert_eq!(
            p.solve_gap(30.0, allow).unwrap(),
            Some((vec![30.0], vec![30.0]))
        );
        assert!(p.solve_gap(31.0, allow).unwrap().is_none());
    }
    #[test]
    fn work_failure_stops_at_the_rejected_reservation() {
        let p = Problem {
            groups: vec![group(0.0, 100.0), group(0.0, 100.0)],
            arcs: vec![Arc {
                from: 0,
                to: 1,
                separate: true,
            }],
        };
        for reject_at in 0..3 {
            let mut charges = Vec::new();
            let result = p.solve_gap(10.0, |units| {
                charges.push(units);
                if charges.len() == reject_at + 1 {
                    Err("cancelled")
                } else {
                    Ok(())
                }
            });
            assert_eq!(result, Err("cancelled"));
            assert_eq!(charges, [4, 1, 1][..=reject_at]);
        }
        assert!(p.solve_gap(10.0, allow).unwrap().is_some());
    }
    #[test]
    fn go_min_max_signed_zero_and_nan() {
        assert_eq!(minimum(0.0, -0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(maximum(-0.0, 0.0).to_bits(), 0.0f64.to_bits());
        assert!(minimum(f64::NAN, 1.0).is_nan());
        assert!(maximum(1.0, f64::NAN).is_nan());
        assert_eq!(minimum(f64::NAN, f64::NEG_INFINITY), f64::NEG_INFINITY);
        assert_eq!(maximum(f64::NAN, f64::INFINITY), f64::INFINITY);
    }
}
