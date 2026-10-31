// Copyright (c) 2010, 2020 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// Translated from elk-go v0.2.0 layered/p5edges/orthogonal/cycle_detector.go.

use std::collections::VecDeque;

/// The dependency kinds used by ELK's hyper-edge cycle detector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DependencyType {
    Regular,
    Critical,
}

/// A directed ordering dependency between two segments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dependency {
    pub source: usize,
    pub target: usize,
    pub weight: usize,
    pub kind: DependencyType,
}

/// Detects the feedback dependencies selected by ELK's
/// `HyperEdgeCycleDetector.detectCycles`.
///
/// The callback supplies `java.util.Random.nextInt(bound)`. Returning `None`
/// leaves a cyclic graph unresolved, which is useful for callers that cannot
/// provide the owning graph's random stream.
pub(crate) fn detect_cycles(
    count: usize,
    dependencies: &[Dependency],
    critical_only: bool,
    mut draw: impl FnMut(usize) -> Option<usize>,
) -> Option<Vec<usize>> {
    let mut incoming = vec![0i64; count];
    let mut outgoing = vec![0i64; count];
    let mut critical_incoming = vec![0i64; count];
    let mut critical_outgoing = vec![0i64; count];
    for dependency in dependencies {
        let weight = dependency.weight as i64;
        if dependency.kind == DependencyType::Critical {
            critical_outgoing[dependency.source] += weight;
            critical_incoming[dependency.target] += weight;
        }
        if !critical_only || dependency.kind == DependencyType::Critical {
            outgoing[dependency.source] += weight;
            incoming[dependency.target] += weight;
        }
    }

    let mut marks = vec![-1i64; count];
    let mut sources = VecDeque::new();
    let mut sinks = VecDeque::new();
    for id in 0..count {
        if outgoing[id] == 0 {
            sinks.push_back(id);
        } else if incoming[id] == 0 {
            sources.push_back(id);
        }
    }

    let mut remaining = count;
    let mark_base = count as i64;
    let mut sink_mark = mark_base - 1;
    let mut source_mark = mark_base + 1;
    while remaining > 0 {
        while let Some(id) = sinks.pop_front() {
            if marks[id] >= 0 {
                continue;
            }
            marks[id] = sink_mark;
            sink_mark -= 1;
            remaining -= 1;
            update_neighbors(
                id,
                &mut marks,
                &mut incoming,
                &mut outgoing,
                &mut critical_incoming,
                &mut critical_outgoing,
                dependencies,
                critical_only,
                &mut sources,
                &mut sinks,
            );
        }
        while let Some(id) = sources.pop_front() {
            if marks[id] >= 0 {
                continue;
            }
            marks[id] = source_mark;
            source_mark += 1;
            remaining -= 1;
            update_neighbors(
                id,
                &mut marks,
                &mut incoming,
                &mut outgoing,
                &mut critical_incoming,
                &mut critical_outgoing,
                dependencies,
                critical_only,
                &mut sources,
                &mut sinks,
            );
        }

        // Java's TreeSet iterates the initial negative marks in reverse
        // segment order. That order stays frozen while positive marks are
        // assigned, so ties retain the same stable candidate sequence.
        let mut maximum = i64::MIN;
        let mut candidates = Vec::new();
        for id in (0..count).rev().filter(|id| marks[*id] < 0) {
            // With criticalOnly=false, ELK gives a segment that can emit a
            // critical dependency without receiving one priority over the
            // ordinary max-outflow heuristic.
            if !critical_only && critical_outgoing[id] > 0 && critical_incoming[id] <= 0 {
                candidates.clear();
                candidates.push(id);
                break;
            }
            let flow = outgoing[id] - incoming[id];
            if flow > maximum {
                candidates.clear();
                maximum = flow;
            }
            if flow == maximum {
                candidates.push(id);
            }
        }
        if candidates.is_empty() {
            break;
        }
        let selected = *candidates.get(draw(candidates.len())?)?;
        marks[selected] = source_mark;
        source_mark += 1;
        remaining -= 1;
        update_neighbors(
            selected,
            &mut marks,
            &mut incoming,
            &mut outgoing,
            &mut critical_incoming,
            &mut critical_outgoing,
            dependencies,
            critical_only,
            &mut sources,
            &mut sinks,
        );
    }

    let shift_base = count as i64 + 1;
    for mark in &mut marks {
        if *mark < mark_base {
            *mark += shift_base;
        }
    }
    Some(
        dependencies
            .iter()
            .enumerate()
            .filter_map(|(index, dependency)| {
                ((!critical_only || dependency.kind == DependencyType::Critical)
                    && marks[dependency.source] > marks[dependency.target])
                    .then_some(index)
            })
            .collect(),
    )
}

#[allow(clippy::too_many_arguments)]
fn update_neighbors(
    id: usize,
    marks: &mut [i64],
    incoming: &mut [i64],
    outgoing: &mut [i64],
    critical_incoming: &mut [i64],
    critical_outgoing: &mut [i64],
    dependencies: &[Dependency],
    critical_only: bool,
    sources: &mut VecDeque<usize>,
    sinks: &mut VecDeque<usize>,
) {
    for dependency in dependencies {
        if critical_only && dependency.kind != DependencyType::Critical {
            continue;
        }
        let weight = dependency.weight as i64;
        if dependency.source == id && marks[dependency.target] < 0 && dependency.weight > 0 {
            incoming[dependency.target] -= weight;
            if dependency.kind == DependencyType::Critical {
                critical_incoming[dependency.target] -= weight;
            }
            if incoming[dependency.target] <= 0 && outgoing[dependency.target] > 0 {
                sources.push_back(dependency.target);
            }
        }
        if dependency.target == id && marks[dependency.source] < 0 && dependency.weight > 0 {
            outgoing[dependency.source] -= weight;
            if dependency.kind == DependencyType::Critical {
                critical_outgoing[dependency.source] -= weight;
            }
            if outgoing[dependency.source] <= 0 && incoming[dependency.source] > 0 {
                sinks.push_back(dependency.source);
            }
        }
    }
}

/// Regular-dependency portion of BreakNonCriticalCycles. The caller owns the
/// graph RNG; an unavailable draw leaves the processor unresolved.
pub(crate) fn break_cycles(
    count: usize,
    dependencies: &[(usize, usize, usize)],
    draw: impl FnMut(usize) -> Option<usize>,
) -> Option<Vec<(usize, usize, usize)>> {
    let typed = dependencies
        .iter()
        .map(|&(source, target, weight)| Dependency {
            source,
            target,
            weight,
            kind: DependencyType::Regular,
        })
        .collect::<Vec<_>>();
    let feedback = detect_cycles(count, &typed, false, draw)?;
    let feedback = feedback
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    Some(
        typed
            .iter()
            .enumerate()
            .filter_map(|(index, dependency)| {
                if feedback.contains(&index) {
                    if dependency.weight == 0 {
                        None
                    } else {
                        Some((dependency.target, dependency.source, dependency.weight))
                    }
                } else {
                    Some((dependency.source, dependency.target, dependency.weight))
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::{Dependency, DependencyType, break_cycles, detect_cycles};

    #[test]
    fn zero_weight_pair_uses_input_order_without_randomness() {
        assert_eq!(
            break_cycles(2, &[(0, 1, 0), (1, 0, 0)], |_| panic!("unexpected draw")),
            Some(vec![(1, 0, 0)])
        );
    }

    #[test]
    fn positive_cycle_requires_and_consumes_a_draw() {
        let edges = [(0, 1, 1), (1, 2, 1), (2, 0, 1)];
        assert!(break_cycles(3, &edges, |_| None).is_none());
        let mut draws = Vec::new();
        let result = break_cycles(3, &edges, |bound| {
            draws.push(bound);
            Some(0)
        })
        .unwrap();
        assert_eq!(draws, vec![3]);
        assert_eq!(result, vec![(0, 1, 1), (2, 1, 1), (2, 0, 1)]);
    }

    #[test]
    fn critical_only_ignores_regular_feedback() {
        let dependencies = [
            Dependency {
                source: 0,
                target: 1,
                weight: 1,
                kind: DependencyType::Regular,
            },
            Dependency {
                source: 1,
                target: 0,
                weight: 1,
                kind: DependencyType::Regular,
            },
        ];
        assert_eq!(
            detect_cycles(2, &dependencies, true, |_| Some(0)).unwrap(),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn critical_feedback_is_selected_in_critical_only_mode() {
        let dependencies = [
            Dependency {
                source: 0,
                target: 1,
                weight: 1,
                kind: DependencyType::Critical,
            },
            Dependency {
                source: 1,
                target: 0,
                weight: 1,
                kind: DependencyType::Critical,
            },
        ];
        assert_eq!(
            detect_cycles(2, &dependencies, true, |_| Some(0)).unwrap(),
            vec![0]
        );
    }
}
