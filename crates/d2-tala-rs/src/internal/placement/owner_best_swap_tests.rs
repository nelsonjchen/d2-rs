// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::super::go_rng::GoRng;
use super::super::limits::ContextError;
use super::super::{Point, Size};
use super::*;
use crate::{Edge, Graph, Node};
use serde_json::{Value, json};
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
struct Quiet;
impl Context for Quiet {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<ContextError> {
        None
    }
}
struct Observe<'a> {
    name: &'a str,
    polls: Cell<u64>,
    cancel: u64,
    rng: OwnerOptimizerRng,
    draw: Cell<Option<u64>>,
}
impl Context for Observe<'_> {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<ContextError> {
        let poll = self.polls.get() + 1;
        self.polls.set(poll);
        if self.name == "caller-draw-before-shuffle" && poll == 9 {
            self.draw
                .set(Some(self.rng.stream().borrow_mut().float64().to_bits()));
        }
        if poll == self.cancel {
            Some(ContextError::Cancelled)
        } else {
            None
        }
    }
}
fn bits(p: Option<Point>) -> Value {
    json!(p.map(|p| [
        format!("{:016x}", p.x.to_bits()),
        format!("{:016x}", p.y.to_bits())
    ]))
}

#[cfg(feature = "diagnostic-traces")]
#[test]
fn actual_best_swap_source_controls() {
    let mut cases = [
        "compass",
        "improve",
        "regress-second",
        "no-candidate",
        "fixed",
        "present-nil",
        "nil-rng",
        "nil-rng-empty",
        "nil-node",
        "nil-graph",
        "nil-position",
        "nan-position",
        "commit",
        "caller-draw-before-shuffle",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    cases.extend((0..32).map(|n| format!("seed-{n}")));
    cases.extend((0..=240).map(|n| format!("budget-{n}")));
    cases.extend((1..=90).map(|n| format!("cancel-{n}")));
    for name in cases {
        let small = matches!(name.as_str(), "improve" | "regress-second");
        let coords = if small {
            vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 1.0, y: 0.0 },
                Point { x: 2.0, y: 0.0 },
                Point {
                    x: if name == "regress-second" { 3.0 } else { -1.0 },
                    y: 0.0,
                },
            ]
        } else {
            vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 0.0, y: -1.0 },
                Point { x: 0.0, y: 1.0 },
                Point { x: -1.0, y: 0.0 },
                Point { x: 1.0, y: 0.0 },
                Point { x: 3.0, y: 2.0 },
                Point { x: -3.0, y: -2.0 },
            ]
        };
        let mut input = Graph::default();
        let nodes = coords
            .iter()
            .enumerate()
            .map(|(i, _)| {
                input.add_node(Node::new(
                    ((b'A' + i as u8) as char).to_string(),
                    Size {
                        width: 10.0,
                        height: 20.0,
                    },
                ))
            })
            .collect::<Vec<_>>();
        let pairs = if small {
            vec![(0, 2), (1, 3)]
        } else {
            vec![(0, 5), (1, 6), (2, 6), (3, 6), (4, 6)]
        };
        let edges = pairs
            .iter()
            .map(|&(a, b)| {
                input.add_edge(Edge {
                    source: nodes[a],
                    target: nodes[b],
                })
            })
            .collect::<Vec<_>>();
        let mut workspace = ArenaGraph::from_input(&input);
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        owner.cell_size = 10.0;
        for (i, &node) in nodes.iter().enumerate() {
            workspace.nodes[node.0 as usize].tala_id = i as i64 + 1;
            owner.nodes[node.0 as usize].tala_id = i as i64 + 1;
            let p = owner.allocate_owner_node_point(Some(coords[i]));
            owner.bind_owner_node_point(node, p).unwrap();
            let incident = pairs
                .iter()
                .enumerate()
                .filter(|(_, (a, b))| *a == i || *b == i)
                .map(|(e, _)| Some(edges[e]))
                .collect::<Vec<_>>();
            let h = owner
                .allocate_owner_incident_header(&incident, incident.len(), true)
                .unwrap();
            owner.set_owner_node_incident_header(node, h).unwrap();
        }
        let seed = name
            .strip_prefix("seed-")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(1);
        let rng = OwnerOptimizerRng::new(GoRng::new(seed));
        let optim = OwnerSizelessOptimizer::new(
            &mut owner,
            Some(OwnerGraphId(0)),
            if matches!(name.as_str(), "nil-rng" | "nil-rng-empty") {
                None
            } else {
                Some(rng.clone())
            },
            Some(&Quiet),
        )
        .unwrap();
        if matches!(name.as_str(), "no-candidate" | "nil-rng-empty") {
            optim.occupied().borrow().clear();
        }
        if name == "fixed" {
            owner.nodes[nodes[1].0 as usize].fixed_top_left = Some(Point::default());
        }
        if name == "present-nil" {
            optim.occupied().borrow().insert(coords[1], None);
        }
        if name == "nil-graph" {
            owner
                .set_owner_node_graph_reference(nodes[0], None)
                .unwrap();
        }
        if name == "nil-position" {
            owner.clear_position(nodes[0]);
        }
        if name == "nan-position" {
            owner.set_position(
                nodes[0],
                Point {
                    x: f64::from_bits(0x7ff8000000000001),
                    y: 0.0,
                },
            );
        }
        assert_eq!(
            owner
                .owner_graph_direction_entry(OwnerGraphId(0), None)
                .unwrap(),
            Some(super::super::Orientation::Bottom)
        );
        assert!(
            owner
                .edge_records
                .iter()
                .all(|e| !e.source_arrow && !e.target_arrow)
        );
        let mapping = CanonicalScoreMapping::new(
            owner.topology_arena_id().unwrap(),
            nodes.clone(),
            edges.clone(),
        );
        let retained = nodes
            .iter()
            .map(|&n| owner.owner_node_point(n).unwrap())
            .collect::<Vec<_>>();
        let context = Observe {
            name: &name,
            polls: Cell::new(0),
            cancel: name
                .strip_prefix("cancel-")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            rng: rng.clone(),
            draw: Cell::new(None),
        };
        let limit = name
            .strip_prefix("budget-")
            .and_then(|s| s.parse().ok())
            .unwrap_or(100000);
        let mut used = 0;
        let mut method_used = 0;
        let mut best = Value::Null;
        let mut error = Value::Null;
        let mut panic_value = Value::Null;
        match OptimizationWorkGuard::new_with_owner(Some(&context), "control", limit, &mut owner) {
            Err(e) => error = json!(e.to_string()),
            Ok(mut guard) => {
                if name == "caller-draw-before-shuffle" {
                    guard.add_with_owner(43, &mut owner).unwrap();
                }
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let candidate = optim.best_swap_candidate_guarded(
                        &workspace,
                        &mut owner,
                        &mapping,
                        if name == "nil-node" {
                            None
                        } else {
                            Some(nodes[0])
                        },
                        &context,
                        &mut guard,
                    )?;
                    method_used = guard.used();
                    if name == "commit" {
                        if let Some(candidate) = candidate {
                            optim.commit_swap_guarded(
                                &mut owner,
                                Some(nodes[0]),
                                Some(candidate),
                                &mut guard,
                            )?;
                        }
                    }
                    Ok::<_, OwnerPlacementError>(candidate)
                }));
                used = guard.used();
                if method_used == 0 {
                    method_used = used;
                }
                match result {
                    Ok(Ok(candidate)) => {
                        best =
                            json!(candidate.map(|n| owner.nodes[n.0 as usize].external_id.clone()));
                    }
                    Ok(Err(e)) => error = json!(e.to_string()),
                    Err(p) => {
                        panic_value = json!(
                            p.downcast_ref::<&str>()
                                .copied()
                                .or_else(|| p.downcast_ref::<String>().map(String::as_str))
                                .unwrap_or("unknown")
                        )
                    }
                }
            }
        }
        let next = format!("{:016x}", rng.stream().borrow_mut().float64().to_bits());
        let same = nodes
            .iter()
            .zip(&retained)
            .map(|(&n, p)| owner.owner_node_point(n).unwrap().same_allocation(p))
            .collect::<Vec<_>>();
        let row = json!({"case":name,"best":best,"error":error,"panic":panic_value,"used":used,"method_used":method_used,"polls":context.polls.get(),"points":nodes.iter().map(|&n|bits(owner.position(n))).collect::<Vec<_>>(),"retained":retained.iter().map(|p|bits(p.get())).collect::<Vec<_>>(),"same_retained":same,"next_rng":next,"context_draw":context.draw.get().map(|b|format!("{b:016x}"))});
        println!("OWNER_BEST_SWAP_CONTROL {row}");
    }
}

#[test]
fn source_float_comparison_controls() {
    for (name, a, b) in [
        ("finite-tie", 1.0, 1.0),
        ("inside-band", 0.0, 0.00009999),
        ("boundary", 0.0, 0.0001),
        ("reverse", 0.0001, 0.0),
        ("zeros", -0.0, 0.0),
        ("positive-infinities", f64::INFINITY, f64::INFINITY),
        ("negative-infinities", f64::NEG_INFINITY, f64::NEG_INFINITY),
        ("infinite-less", f64::NEG_INFINITY, 1.0),
        ("infinite-greater", f64::INFINITY, 1.0),
        ("nan-first", f64::NAN, 1.0),
        ("nan-second", 1.0, f64::NAN),
        ("both-nan", f64::NAN, f64::NAN),
    ] {
        let result = match precision_compare(a, b) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        println!(
            "OWNER_FLOAT_COMPARE_CONTROL {}",
            json!({"case":name,"result":result})
        );
    }
}
