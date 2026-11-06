// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Component partitioning and candidate search from applyChannelProblem.
//! Graph point proposals, quality checks and atomic commit are supplied by the
//! route adapter. No coordinate mutation occurs while candidates are evaluated.
use super::channel_solver::{Arc, Problem, maximum, minimum};

const EPSILON: f64 = 1e-6;

pub(super) struct Component {
    pub indices: Vec<usize>,
    pub problem: Problem,
}

fn root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

pub(super) fn components(problem: &Problem) -> Vec<Component> {
    let mut parent: Vec<_> = (0..problem.groups.len()).collect();
    for arc in &problem.arcs {
        let to = root(&mut parent, arc.to);
        let from = root(&mut parent, arc.from);
        parent[to] = from;
    }
    let mut by_root = vec![None; parent.len()];
    let mut partitions: Vec<Vec<usize>> = Vec::new();
    for i in 0..parent.len() {
        let id = root(&mut parent, i);
        let index = *by_root[id].get_or_insert_with(|| {
            partitions.push(Vec::new());
            partitions.len() - 1
        });
        partitions[index].push(i);
    }
    partitions
        .into_iter()
        .map(|indices| {
            let mut local_indices = vec![None; parent.len()];
            for (local, &global) in indices.iter().enumerate() {
                local_indices[global] = Some(local);
            }
            let groups = indices.iter().map(|&i| problem.groups[i].clone()).collect();
            let arcs = problem
                .arcs
                .iter()
                .filter_map(|arc| {
                    local_indices[arc.from].map(|from| Arc {
                        from,
                        to: local_indices[arc.to].expect("arc connects its component"),
                        separate: arc.separate,
                    })
                })
                .collect();
            Component {
                indices,
                problem: Problem { groups, arcs },
            }
        })
        .collect()
}

pub(super) trait Evaluator {
    type Error;
    type Proposal;
    fn reserve(&mut self, units: u64) -> Result<(), Self::Error>;
    /// None is the upstream incompatible-point-alias result.
    fn proposal(&mut self, positions: &[f64]) -> Result<Option<Self::Proposal>, Self::Error>;
    /// None measures the current complete graph, including earlier components.
    fn wire_length(&mut self, proposal: Option<&Self::Proposal>) -> Result<f64, Self::Error>;
    fn safe(&mut self, proposal: &Self::Proposal) -> Result<bool, Self::Error>;
}

pub(super) fn select<E: Evaluator>(
    problem: &Problem,
    positions: &[f64],
    evaluator: &mut E,
) -> Result<Option<E::Proposal>, E::Error> {
    let old_gap = maximum(0.0, problem.gap(positions));
    let mut left = old_gap;
    let mut right = f64::INFINITY;
    for group in &problem.groups {
        right = minimum(right, (group.upper - group.lower) / 2.0);
    }
    let mut iteration = 0;
    while iteration < 32 && right - left > EPSILON {
        iteration += 1;
        let mid = (left + right) / 2.0;
        if problem
            .solve_gap(mid, |units| evaluator.reserve(units))?
            .is_some()
        {
            left = mid;
        } else {
            right = mid;
        }
    }
    let mut best_length = evaluator.wire_length(None)?;
    let mut best_gap = old_gap;
    let mut best = None;
    for gap in [old_gap, left] {
        let Some((earliest, latest)) = problem.solve_gap(gap, |units| evaluator.reserve(units))?
        else {
            continue;
        };
        for fraction in [0.0, 1.0, 0.5] {
            let candidate: Vec<_> = earliest
                .iter()
                .zip(&latest)
                .map(|(lo, hi)| lo * (1.0 - fraction) + hi * fraction)
                .collect();
            let Some(proposal) = evaluator.proposal(&candidate)? else {
                continue;
            };
            let length = evaluator.wire_length(Some(&proposal))?;
            let actual_gap = problem.gap(&candidate);
            if length > best_length + EPSILON
                || ((length - best_length).abs() <= EPSILON && actual_gap < best_gap + 1.0)
            {
                continue;
            }
            if !evaluator.safe(&proposal)? {
                continue;
            }
            best = Some(proposal);
            best_length = length;
            best_gap = actual_gap;
        }
    }
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::super::channel_solver::Group;
    use super::*;
    fn group() -> Group {
        Group {
            lower: 0.0,
            upper: 100.0,
            node_clearance: None,
        }
    }
    #[test]
    fn disconnected_components_retain_encounter_and_arc_order() {
        let p = Problem {
            groups: (0..5).map(|_| group()).collect(),
            arcs: vec![
                Arc {
                    from: 0,
                    to: 3,
                    separate: true,
                },
                Arc {
                    from: 1,
                    to: 2,
                    separate: false,
                },
            ],
        };
        let c = components(&p);
        assert_eq!(
            c.iter().map(|c| c.indices.clone()).collect::<Vec<_>>(),
            vec![vec![0, 3], vec![1, 2], vec![4]]
        );
        assert_eq!(c[0].problem.arcs[0].to, 1);
        assert!(!c[1].problem.arcs[0].separate);
    }
    struct Route {
        visited: Vec<Vec<f64>>,
        unsafe_candidates: bool,
        fail_work: bool,
        baseline: f64,
    }
    impl Evaluator for Route {
        type Error = &'static str;
        type Proposal = Vec<f64>;
        fn reserve(&mut self, _: u64) -> Result<(), Self::Error> {
            if self.fail_work {
                Err("cancelled")
            } else {
                Ok(())
            }
        }
        fn proposal(&mut self, p: &[f64]) -> Result<Option<Vec<f64>>, Self::Error> {
            self.visited.push(p.to_vec());
            Ok(Some(p.to_vec()))
        }
        fn wire_length(&mut self, p: Option<&Vec<f64>>) -> Result<f64, Self::Error> {
            Ok(p.map_or(self.baseline, |p| 100.0 + 2.0 * p[0]))
        }
        fn safe(&mut self, _: &Vec<f64>) -> Result<bool, Self::Error> {
            Ok(!self.unsafe_candidates)
        }
    }
    #[test]
    fn shortening_preserves_existing_gap_and_extremal_order() {
        let p = Problem {
            groups: vec![group()],
            arcs: vec![],
        };
        let mut e = Route {
            visited: vec![],
            unsafe_candidates: false,
            fail_work: false,
            baseline: 240.0,
        };
        let best = select(&p, &[70.0], &mut e).unwrap().unwrap();
        assert_eq!(best, vec![30.0]);
        assert_eq!(&e.visited[..3], &[vec![30.0], vec![70.0], vec![50.0]]);
        assert_eq!(e.visited.len(), 6);
    }
    #[test]
    fn unsafe_or_cancelled_search_returns_no_committable_update() {
        let p = Problem {
            groups: vec![group()],
            arcs: vec![],
        };
        let mut e = Route {
            visited: vec![],
            unsafe_candidates: true,
            fail_work: false,
            baseline: 240.0,
        };
        assert!(select(&p, &[70.0], &mut e).unwrap().is_none());
        e.visited.clear();
        e.fail_work = true;
        assert_eq!(select(&p, &[70.0], &mut e), Err("cancelled"));
        assert!(e.visited.is_empty());
    }
}
