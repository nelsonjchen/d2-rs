// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::*;
use crate::internal::shared_topology::{OwnerHerd, OwnerHerdMap};
fn observe_herd_map(owner: &ArenaGraph, map: &OwnerHerdMap) -> Value {
    let mut keys = map
        .keys()
        .into_iter()
        .map(|n| n.map(|n| owner.nodes[n.0 as usize].external_id.clone()))
        .collect::<Vec<_>>();
    keys.sort();
    json!({"nil":map.is_nil(),"len":map.len(),"keys":keys})
}
fn observe_herd(owner: &ArenaGraph, herd: &OwnerHerd) -> Value {
    let fields = owner.owner_herd_fields(herd).unwrap();
    json!({"orientation":fields.orientation,"value":format!("{:016x}",fields.value.to_bits()),"same":observe_herd_map(owner,&fields.same_side),"opposite":observe_herd_map(owner,&fields.opposite_side),"alias_maps":fields.same_side.same_map_allocation(&fields.opposite_side)})
}
struct HerdObserve {
    polls: Cell<u64>,
    cancel: u64,
    panic_at: u64,
    mutation_poll: u64,
    mutated: Cell<bool>,
    herd: OwnerHerd,
    other: OwnerHerd,
    replacement: OwnerHerd,
    replacement_map: RefCell<Option<OwnerHerdMap>>,
    nodes: Vec<NodeId>,
}
impl Context for HerdObserve {
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
        let poll = self.polls.get() + 1;
        self.polls.set(poll);
        if poll == self.mutation_poll {
            self.mutated.set(true);
            let mut fields = owner.owner_herd_fields(&self.herd).unwrap();
            fields.orientation = 41;
            fields.value = 99.;
            owner.set_owner_herd_fields(&self.herd, fields).unwrap();
            owner
                .pair_owner_herd(Some(&self.herd), None, false)
                .unwrap();
            owner
                .pair_owner_herd(Some(&self.herd), Some(self.nodes[0]), true)
                .unwrap();
            let mut fields = owner.owner_herd_fields(&self.herd).unwrap();
            owner
                .delete_owner_herd_pair(&fields.same_side, Some(self.nodes[1]))
                .unwrap();
            let replacement_map = owner.allocate_owner_herd_map(true);
            fields.same_side = replacement_map.clone();
            fields.opposite_side = owner.allocate_owner_herd_map(false);
            owner.set_owner_herd_fields(&self.herd, fields).unwrap();
            owner
                .pair_owner_herd(Some(&self.herd), Some(self.nodes[3]), false)
                .unwrap();
            *self.replacement_map.borrow_mut() = Some(replacement_map);
            owner
                .set_owner_node_herd(self.nodes[0], Some(self.replacement.clone()))
                .unwrap();
            owner.set_owner_node_herd(self.nodes[1], None).unwrap();
            let mut fields = owner.owner_herd_fields(&self.other).unwrap();
            fields.orientation = -9;
            fields.value = -5.;
            owner.set_owner_herd_fields(&self.other, fields).unwrap();
        }
        if poll == self.panic_at {
            panic!("caller panic")
        };
        (poll == self.cancel).then_some(ContextError::Cancelled)
    }
}
#[cfg(feature = "diagnostic-traces")]
#[test]
fn complete_herd_optimizer_source_controls() {
    let mut cases = [
        "ordinary",
        "default",
        "nil-herd",
        "nil-maps",
        "aliased-maps",
        "all-shared",
        "distinct",
        "special-values",
        "duplicate-ids",
        "outside-herd-mutate-cancel-60",
        "capture-boundary-cancel",
        "capture-boundary-panic",
        "capture-boundary-success",
        "mutate-success-60",
        "unreferenced-mutate-cancel-60",
        "all-shared-mutate-cancel-60",
        "distinct-mutate-cancel-60",
        "aliased-maps-mutate-cancel-60",
        "nil-maps-mutate-cancel-60",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    cases.extend((0..=500).map(|n| format!("budget-{n}")));
    cases.extend((1..=160).map(|n| format!("cancel-{n}")));
    cases.extend((1..=160).map(|n| format!("mutate-cancel-{n}")));
    cases.extend([1, 2, 3, 20, 60, 120, 147, 500].map(|n| format!("mutate-panic-{n}")));
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
        if name == "duplicate-ids" {
            owner.nodes[1].tala_id = 1;
            workspace.nodes[1].tala_id = 1;
        }
        if name.starts_with("capture-boundary") {
            let mut slots = vec![Some(nodes[0]); 60];
            slots.extend(nodes[1..].iter().copied().map(Some));
            let header = owner
                .allocate_owner_member_header(&slots, slots.len())
                .unwrap();
            owner
                .set_owner_graph_member_header(OwnerGraphId(0), header)
                .unwrap();
        }
        if name.starts_with("outside-herd") {
            let header = owner
                .allocate_owner_member_header(&[Some(nodes[0]), Some(nodes[1])], 2)
                .unwrap();
            owner
                .set_owner_graph_member_header(OwnerGraphId(0), header)
                .unwrap();
            owner
                .containers
                .insert(Some(nodes[0]), vec![nodes[2], nodes[3]]);
        }
        let rng = OwnerOptimizerRng::new(GoRng::new(1));
        let optim = OwnerSizelessOptimizer::new(
            &mut owner,
            Some(OwnerGraphId(0)),
            Some(rng.clone()),
            Some(&Quiet),
        )
        .unwrap();
        let herd = owner.new_owner_herd();
        let replacement = owner.new_owner_herd();
        let other = owner.new_owner_herd();
        let mut fields = owner.owner_herd_fields(&herd).unwrap();
        if name != "default" {
            fields.orientation = 4;
            fields.value = 10.;
        }
        owner.set_owner_herd_fields(&herd, fields).unwrap();
        if name != "default" {
            owner
                .pair_owner_herd(Some(&herd), Some(nodes[1]), false)
                .unwrap();
            owner
                .pair_owner_herd(Some(&herd), Some(nodes[2]), true)
                .unwrap();
        }
        if name.starts_with("aliased-maps") {
            let mut f = owner.owner_herd_fields(&herd).unwrap();
            f.opposite_side = f.same_side.clone();
            owner.set_owner_herd_fields(&herd, f).unwrap();
        }
        if name.starts_with("nil-maps") {
            let mut f = owner.owner_herd_fields(&herd).unwrap();
            f.same_side = owner.allocate_owner_herd_map(false);
            f.opposite_side = owner.allocate_owner_herd_map(false);
            owner.set_owner_herd_fields(&herd, f).unwrap();
        }
        if name == "special-values" {
            let mut f = owner.owner_herd_fields(&herd).unwrap();
            f.orientation = i64::MAX;
            f.value = f64::from_bits(0x7ff0000000000123);
            owner.set_owner_herd_fields(&herd, f).unwrap();
        }
        let mut f = owner.owner_herd_fields(&replacement).unwrap();
        f.orientation = 8;
        f.value = 7.;
        owner.set_owner_herd_fields(&replacement, f).unwrap();
        let mut f = owner.owner_herd_fields(&other).unwrap();
        f.orientation = 5;
        f.value = 20.;
        owner.set_owner_herd_fields(&other, f).unwrap();
        let mut assigned = vec![
            Some(herd.clone()),
            Some(herd.clone()),
            Some(herd.clone()),
            Some(other.clone()),
        ];
        if name.starts_with("all-shared") {
            assigned[3] = Some(herd.clone());
        }
        if name.starts_with("distinct") {
            assigned[1] = Some(owner.copy_owner_herd(Some(&herd)).unwrap());
            assigned[2] = Some(owner.copy_owner_herd(Some(&herd)).unwrap());
        }
        if name == "nil-herd" || name.starts_with("unreferenced") {
            assigned[0] = None;
            assigned[1] = None;
            assigned[2] = None;
        }
        for (node, ha) in nodes.iter().copied().zip(&assigned) {
            owner.set_owner_node_herd(node, ha.clone()).unwrap();
        }
        let fields = owner.owner_herd_fields(&herd).unwrap();
        let old_same = fields.same_side;
        let old_opposite = fields.opposite_side;
        let retained = nodes
            .iter()
            .map(|&n| owner.owner_node_point(n).unwrap())
            .collect::<Vec<_>>();
        let occupied = optim.occupied().borrow().clone();
        let mutation_poll = if name.starts_with("capture-boundary") {
            3
        } else if name.contains("mutate-") {
            name.rsplit('-').next().unwrap().parse().unwrap()
        } else {
            0
        };
        let cancel = if name == "capture-boundary-cancel" {
            4
        } else if name.contains("mutate-cancel-") {
            mutation_poll
        } else {
            name.strip_prefix("cancel-")
                .and_then(|n| n.parse().ok())
                .unwrap_or(0)
        };
        let panic_at = if name == "capture-boundary-panic" {
            4
        } else if name.contains("mutate-panic-") {
            mutation_poll
        } else {
            0
        };
        let context = HerdObserve {
            polls: Cell::new(0),
            cancel,
            panic_at,
            mutation_poll,
            mutated: Cell::new(false),
            herd: herd.clone(),
            other: other.clone(),
            replacement: replacement.clone(),
            replacement_map: RefCell::new(None),
            nodes: nodes.clone(),
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
            optim.optimize_with_limit(&mut workspace, &mut session, &context, temp, limit)
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
        let entries = map_entries(&optim.occupied().borrow(), &owner);
        let scratch = optim.take_mutation_scratch();
        let released = scratch.captured_nodes().len() == 0
            && !scratch.is_complete()
            && scratch.cost_snapshot().is_none();
        assert!(released);
        optim.retain_mutation_scratch(scratch);
        println!(
            "OWNER_HERD_LOOP_CONTROL {}",
            json!({"case":name,"herds":[observe_herd(&owner,&herd),observe_herd(&owner,&replacement),observe_herd(&owner,&other)],"assigned_retained":assigned.iter().map(|h|h.as_ref().map(|h|observe_herd(&owner,h))).collect::<Vec<_>>(),"node_herds":nodes.iter().map(|&n|owner.owner_node_herd(n).unwrap().as_ref().map(|ha|if ha.same_allocation(&herd){"original"}else if ha.same_allocation(&replacement){"replacement"}else if ha.same_allocation(&other){"other"}else{"copy"})).collect::<Vec<_>>(),"old_same":observe_herd_map(&owner,&old_same),"old_opposite":observe_herd_map(&owner,&old_opposite),"same_old_maps":({let fields=owner.owner_herd_fields(&herd).unwrap();[fields.same_side.same_map_allocation(&old_same),fields.opposite_side.same_map_allocation(&old_opposite)]}),"replacement_map":context.replacement_map.borrow().as_ref().map(|m|observe_herd_map(&owner,m)),"mutated":context.mutated.get(),"old_occupied":map_entries(&occupied,&owner),"replacement_occupied":Value::Null,"sizes":nodes.iter().map(|n|[format!("{:016x}",owner.nodes[n.0 as usize].rect.size.width.to_bits()),format!("{:016x}",owner.nodes[n.0 as usize].rect.size.height.to_bits())]).collect::<Vec<_>>(),"error":error,"panic":panic_value,"polls":context.polls.get(),"points":nodes.iter().map(|&n|bits(owner.position(n))).collect::<Vec<_>>(),"retained":retained.iter().map(|p|bits(p.get())).collect::<Vec<_>>(),"same_retained":same,"occupied":entries,"same_map":optim.occupied().borrow().same_map_allocation(&occupied),"nil_map":optim.occupied().borrow().is_nil(),"scratch_released":released,"next_rng":format!("{:016x}",rng.stream().borrow_mut().float64().to_bits())})
        );
    }
}

#[test]
fn source_herd_allocation_copy_and_pair_controls() {
    for name in [
        "new",
        "copy",
        "alias-copy",
        "nil-copy",
        "nil-same-copy",
        "nil-opposite-copy",
        "nil-receiver-copy",
        "nil-receiver-pair",
        "nil-map-pair",
        "nil-key",
        "duplicate-pairs",
        "unknown-orientation",
        "special-value",
        "duplicate-ids",
    ] {
        let mut input = Graph::default();
        let nodes = [
            input.add_node(Node::new("A", Size::default())),
            input.add_node(Node::new("B", Size::default())),
        ];
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        owner.nodes[0].tala_id = 1;
        owner.nodes[1].tala_id = if name == "duplicate-ids" { 1 } else { 2 };
        let herd = owner.new_owner_herd();
        if name != "new" {
            owner
                .pair_owner_herd(Some(&herd), Some(nodes[0]), false)
                .unwrap();
            owner
                .pair_owner_herd(Some(&herd), Some(nodes[1]), true)
                .unwrap();
            let mut f = owner.owner_herd_fields(&herd).unwrap();
            f.orientation = 4;
            f.value = 10.;
            if name == "alias-copy" {
                f.opposite_side = f.same_side.clone();
            }
            if matches!(name, "nil-copy" | "nil-same-copy" | "nil-map-pair") {
                f.same_side = owner.allocate_owner_herd_map(false);
            }
            if matches!(name, "nil-copy" | "nil-opposite-copy") {
                f.opposite_side = owner.allocate_owner_herd_map(false);
            }
            if name == "unknown-orientation" {
                f.orientation = 42;
                f.value = -0.0;
            }
            if name == "special-value" {
                f.orientation = i64::MIN;
                f.value = f64::from_bits(0xfff0000000000123);
            }
            owner.set_owner_herd_fields(&herd, f).unwrap();
            if name == "nil-key" {
                owner.pair_owner_herd(Some(&herd), None, false).unwrap();
            }
            if name == "duplicate-pairs" {
                owner
                    .pair_owner_herd(Some(&herd), Some(nodes[0]), false)
                    .unwrap();
                owner
                    .pair_owner_herd(Some(&herd), Some(nodes[0]), false)
                    .unwrap();
            }
            if name == "duplicate-ids" {
                owner
                    .pair_owner_herd(Some(&herd), Some(nodes[1]), false)
                    .unwrap();
            }
        }
        let mut copied = None;
        let mut shares = None;
        let outcome = catch_unwind(AssertUnwindSafe(|| match name {
            "new" => {}
            "nil-receiver-copy" => {
                let _ = owner.copy_owner_herd(None).unwrap();
            }
            "nil-receiver-pair" => {
                owner.pair_owner_herd(None, None, false).unwrap();
            }
            "nil-map-pair" => {
                owner
                    .pair_owner_herd(Some(&herd), Some(nodes[0]), false)
                    .unwrap();
            }
            _ => {
                let next = owner.copy_owner_herd(Some(&herd)).unwrap();
                let old = owner.owner_herd_fields(&herd).unwrap();
                let f = owner.owner_herd_fields(&next).unwrap();
                shares = Some([
                    old.same_side.same_map_allocation(&f.same_side),
                    old.opposite_side.same_map_allocation(&f.opposite_side),
                ]);
                if !f.same_side.is_nil() {
                    owner.pair_owner_herd(Some(&next), None, false).unwrap();
                }
                if !f.opposite_side.is_nil() {
                    owner
                        .pair_owner_herd(Some(&next), Some(nodes[0]), true)
                        .unwrap();
                }
                owner
                    .delete_owner_herd_pair(&old.same_side, Some(nodes[0]))
                    .unwrap();
                let mut old = old;
                old.orientation = -9;
                old.value = 33.;
                owner.set_owner_herd_fields(&herd, old).unwrap();
                copied = Some(next);
            }
        }));
        let panic_value = outcome.err().map(|p| {
            p.downcast_ref::<&str>()
                .copied()
                .or_else(|| p.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown")
                .to_owned()
        });
        println!(
            "OWNER_HERD_API_CONTROL {}",
            json!({"case":name,"panic":panic_value,"original":observe_herd(&owner,&herd),"copy":copied.as_ref().map(|h|observe_herd(&owner,h)),"same_record":copied.as_ref().map(|h|h.same_allocation(&herd)),"shares_original_maps":shares})
        );
    }
}

#[test]
fn herd_handles_preserve_aliases_and_reject_foreign_mutation() {
    let mut input = Graph::default();
    let a = input.add_node(Node::new("A", Size::default()));
    let b = input.add_node(Node::new("B", Size::default()));
    let mut owner = ArenaGraph::from_input(&input);
    owner.initialize_shared_topology();
    let herd = owner.new_owner_herd();
    owner.set_owner_node_herd(a, Some(herd.clone())).unwrap();
    owner.set_owner_node_herd(b, Some(herd.clone())).unwrap();
    owner.pair_owner_herd(Some(&herd), Some(a), false).unwrap();
    let mut detached = owner.clone();
    let copied = detached.owner_node_herd(a).unwrap().unwrap();
    assert!(!copied.same_allocation(&herd));
    assert!(copied.same_allocation(&detached.owner_node_herd(b).unwrap().unwrap()));
    let before = observe_herd(&owner, &herd);
    assert!(detached.set_owner_node_herd(b, Some(herd.clone())).is_err());
    detached
        .pair_owner_herd(Some(&copied), Some(b), false)
        .unwrap();
    assert_eq!(observe_herd(&owner, &herd), before);
    assert_eq!(
        detached.owner_herd_fields(&copied).unwrap().same_side.len(),
        2
    );
    let mut foreign = ArenaGraph::from_input(&input);
    foreign.initialize_shared_topology();
    assert!(foreign.set_owner_node_herd(a, Some(herd.clone())).is_err());
    assert_eq!(observe_herd(&owner, &herd), before);
}
