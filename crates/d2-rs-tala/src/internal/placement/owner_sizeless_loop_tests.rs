// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
#[cfg(feature = "diagnostic-traces")]
use super::super::go_rng::GoRng;
use super::super::limits::ContextError;
use super::super::occupied::OccupiedPointMap;
#[cfg(feature = "diagnostic-traces")]
use super::super::owner_sizeless_constructor::OwnerOptimizerRng;
use super::super::shared_topology::OwnerNodePoint;
#[cfg(feature = "diagnostic-traces")]
use super::super::{CanonicalScoreMapping, OwnerGraphId};
use super::super::{Point, Size};
use super::*;
#[cfg(feature = "diagnostic-traces")]
use crate::Edge;
use crate::{Graph, Node};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
#[cfg(feature = "diagnostic-traces")]
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
    polls: Cell<u64>,
    cancel: u64,
    panic_at: u64,
    mutation_poll: u64,
    occupied: &'a RefCell<OccupiedPointMap>,
    original: OccupiedPointMap,
    point: OwnerNodePoint,
    replacement: RefCell<Option<OccupiedPointMap>>,
    mutated: Cell<bool>,
}
impl Context for Observe<'_> {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<ContextError> {
        let poll = self.polls.get() + 1;
        self.polls.set(poll);
        if poll == self.panic_at {
            panic!("caller panic")
        };
        (poll == self.cancel).then_some(ContextError::Cancelled)
    }
    #[cfg(feature = "diagnostic-traces")]
    fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
        if self.polls.get() + 1 == self.mutation_poll {
            self.mutated.set(true);
            owner
                .write_owner_point(&self.point, Point { x: 99., y: 98. })
                .unwrap();
            owner.set_position(NodeId(0), Point { x: 77., y: 76. });
            owner.nodes[0].rect.size = Size {
                width: 88.,
                height: 87.,
            };
            self.original
                .insert(Point { x: 8., y: 8. }, Some(NodeId(0)));
            let replacement = OccupiedPointMap::new();
            replacement.insert(Point { x: 9., y: 9. }, None);
            *self.replacement.borrow_mut() = Some(replacement.clone());
            *self.occupied.borrow_mut() = replacement;
        }
        self.error()
    }
}
fn map_entries(
    map: &OccupiedPointMap,
    owner: &ArenaGraph,
) -> Vec<(String, String, Option<String>)> {
    let mut entries = map
        .observed_entries()
        .into_iter()
        .map(|(p, n)| {
            (
                format!("{:016x}", p.x.to_bits()),
                format!("{:016x}", p.y.to_bits()),
                n.map(|n| owner.nodes[n.0 as usize].external_id.clone()),
            )
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}
fn bits(p: Option<Point>) -> Value {
    json!(p.map(|p| [
        format!("{:016x}", p.x.to_bits()),
        format!("{:016x}", p.y.to_bits())
    ]))
}
#[cfg(feature = "diagnostic-traces")]
#[test]
fn full_optimizer_source_controls() {
    let mut cases = [
        "ordinary",
        "temperature",
        "nil-rng",
        "nil-position",
        "fixed",
        "present-nil",
        "panic-60",
        "panic-120",
        "nil-map",
        "capture-mutate-cancel",
        "finish-mutate-cancel",
        "outer-mutate-panic",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    cases.extend((0..16).map(|n| format!("seed-{n}")));
    cases.extend((0..=700).map(|n| format!("budget-{n}")));
    cases.extend((1..=200).map(|n| format!("cancel-{n}")));
    for name in cases {
        let coords = [
            Point { x: 0., y: 0. },
            Point { x: 4., y: 0. },
            Point { x: 0., y: 4. },
            Point { x: 5., y: 5. },
        ];
        let pairs = [(0, 1), (0, 2), (1, 3), (2, 3)];
        let mut input = Graph::default();
        let nodes = (0..4)
            .map(|i| {
                input.add_node(Node::new(
                    ((b'A' + i) as char).to_string(),
                    Size {
                        width: 10.,
                        height: 20.,
                    },
                ))
            })
            .collect::<Vec<_>>();
        let edges = pairs
            .iter()
            .map(|&(a, b)| {
                input.add_edge(Edge {
                    source: nodes[a],
                    target: nodes[b],
                })
            })
            .collect::<Vec<_>>();
        let mut owner = ArenaGraph::from_input(&input);
        let mut workspace = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        owner.cell_size = 10.;
        for (i, &node) in nodes.iter().enumerate() {
            owner.nodes[node.0 as usize].tala_id = i as i64 + 1;
            workspace.nodes[node.0 as usize].tala_id = i as i64 + 1;
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
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let rng = OwnerOptimizerRng::new(GoRng::new(seed));
        let optim = OwnerSizelessOptimizer::new(
            &mut owner,
            Some(OwnerGraphId(0)),
            (name != "nil-rng").then(|| rng.clone()),
            Some(&Quiet),
        )
        .unwrap();
        if name == "nil-position" {
            owner.clear_position(nodes[0]);
        }
        if name == "fixed" {
            owner.nodes[nodes[1].0 as usize].fixed_top_left = Some(Point::default());
        }
        if name == "present-nil" {
            optim
                .occupied()
                .borrow()
                .insert(Point { x: 2., y: 2. }, None);
        }
        if name == "nil-map" {
            *optim.occupied().borrow_mut() = OccupiedPointMap::default();
        }
        let retained = nodes
            .iter()
            .map(|&n| owner.owner_node_point(n).unwrap())
            .collect::<Vec<_>>();
        let occupied = optim.occupied().borrow().clone();
        let mutation_poll = match name.as_str() {
            "capture-mutate-cancel" => 2,
            "finish-mutate-cancel" => 147,
            "outer-mutate-panic" => 148,
            _ => 0,
        };
        let context = Observe {
            mutation_poll,
            occupied: optim.occupied(),
            original: occupied.clone(),
            point: retained[0].clone(),
            replacement: RefCell::new(None),
            mutated: Cell::new(false),
            polls: Cell::new(0),
            cancel: name
                .strip_prefix("cancel-")
                .and_then(|s| s.parse().ok())
                .unwrap_or(
                    if name == "capture-mutate-cancel" || name == "finish-mutate-cancel" {
                        mutation_poll
                    } else {
                        0
                    },
                ),
            panic_at: name
                .strip_prefix("panic-")
                .and_then(|s| s.parse().ok())
                .unwrap_or(if name == "outer-mutate-panic" {
                    mutation_poll
                } else {
                    0
                }),
        };
        let limit = name
            .strip_prefix("budget-")
            .and_then(|s| s.parse().ok())
            .unwrap_or(100000);
        let temp = if name == "temperature" { 1.0 } else { 0.0 };
        let mapping = CanonicalScoreMapping::new(
            owner.topology_arena_id().unwrap(),
            nodes.clone(),
            edges.clone(),
        );
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        let result = catch_unwind(AssertUnwindSafe(|| {
            optim.optimize_with_limit(&workspace, &mut session, &context, temp, limit)
        }));
        drop(session);
        let mut error = Value::Null;
        let mut panic_value = Value::Null;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => error = json!(e.to_string()),
            Err(p) => {
                panic_value = json!(
                    p.downcast_ref::<&str>()
                        .copied()
                        .or_else(|| p.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("unknown")
                )
            }
        };
        let same = nodes
            .iter()
            .zip(&retained)
            .map(|(&n, p)| owner.owner_node_point(n).unwrap().same_allocation(p))
            .collect::<Vec<_>>();
        // Only an error return restores; an unwound pass is discarded by the
        // seed recovery, so its allocations are left unspecified.
        if !error.is_null() && name != "capture-mutate-cancel" {
            assert!(
                same.iter().all(|&s| s),
                "{name}: original point allocation must be restored"
            );
        }
        let entries = map_entries(&optim.occupied().borrow(), &owner);
        let scratch = optim.take_mutation_scratch();
        let released = scratch.captured_nodes().len() == 0
            && !scratch.is_complete()
            && scratch.cost_snapshot().is_none();
        assert!(released);
        optim.retain_mutation_scratch(scratch);
        println!(
            "OWNER_OPTIMIZER_LOOP_CONTROL {}",
            json!({"case":name,"mutated":context.mutated.get(),"old_occupied":map_entries(&occupied,&owner),"replacement_occupied":context.replacement.borrow().as_ref().map(|map|map_entries(map,&owner)),"sizes":nodes.iter().map(|n|[format!("{:016x}",owner.nodes[n.0 as usize].rect.size.width.to_bits()),format!("{:016x}",owner.nodes[n.0 as usize].rect.size.height.to_bits())]).collect::<Vec<_>>(),"error":error,"panic":panic_value,"polls":context.polls.get(),"points":nodes.iter().map(|&n|bits(owner.position(n))).collect::<Vec<_>>(),"retained":retained.iter().map(|p|bits(p.get())).collect::<Vec<_>>(),"same_retained":same,"occupied":entries,"same_map":optim.occupied().borrow().same_map_allocation(&occupied),"nil_map":optim.occupied().borrow().is_nil(),"scratch_released":released,"next_rng":format!("{:016x}",rng.stream().borrow_mut().float64().to_bits())})
        );
    }
}

#[path = "owner_group_loop_tests.rs"]
mod group_controls;

#[path = "owner_herd_loop_tests.rs"]
mod herd_controls;
