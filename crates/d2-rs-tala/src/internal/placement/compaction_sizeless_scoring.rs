// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Fixed-topology compaction translations share the upstream scorer sweep
//! contract (edge_length_scorer.go:15–23). Geometry remains live at each score.
use super::scoring::OwnerSizelessEdgeLengthScorer;
use super::scoring_context::ScoreContext;
use super::*;

pub(super) struct Scorers(Option<Vec<OwnerSizelessEdgeLengthScorer>>);

impl Scorers {
    pub(super) fn new(nodes: &[NodeId], stable: bool, context: &dyn limits::Context) -> Self {
        Self(stable.then(|| {
            nodes
                .iter()
                .map(|&n| {
                    let mut scorer = OwnerSizelessEdgeLengthScorer::new(n, true);
                    // shiftSubgraphs only translates geometry, and `stable`
                    // guarantees that workspace polls leave topology intact.
                    // With stable owner polls too, edge_length_scorer.go's
                    // incident preparation needs no full topology snapshot.
                    // Initial owner/workspace admission still runs lazily.
                    if ScoreContext::new(context).stable_owner() {
                        scorer.assume_stable_topology();
                    }
                    scorer
                })
                .collect()
        }))
    }

    pub(super) fn score(
        &mut self,
        graph: &mut ArenaGraph,
        nodes: &[NodeId],
        session: &mut Option<&mut OwnerScoreSession<'_>>,
        context: &dyn limits::Context,
    ) -> Result<f64, limits::OptimizationError> {
        let Some(scorers) = &mut self.0 else {
            return compaction_scoring::nodes_edge_length(graph, nodes, false, session, context);
        };
        let session = session
            .as_deref_mut()
            .expect("stable compaction owner session");
        session
            .update_current_member_positions(graph)
            .map_err(|e| limits::OptimizationError::Invalid(e.to_string()))?;
        let score_context = ScoreContext::new(context);
        score_context
            .check()
            .map_err(compaction_candidates::optimization_error)?;
        let mut total = 0.0;
        let mut admitted = false;
        for scorer in scorers {
            // A successful score admitted the entire workspace and mapping.
            // This subgraph changes only geometry; stable owner polls keep
            // that admission valid for the following scorers as well.
            if admitted && score_context.stable_owner() {
                scorer.reuse_admission();
            }
            total += session
                .score_sizeless_owner(scorer, graph, score_context)
                .map_err(compaction_candidates::optimization_error)?;
            admitted = true;
        }
        score_context
            .check()
            .map_err(compaction_candidates::optimization_error)?;
        Ok(total)
    }
}

impl Drop for Scorers {
    fn drop(&mut self) {
        if let Some(scorers) = &mut self.0 {
            for scorer in scorers {
                scorer.close();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use std::cell::Cell;

    #[test]
    fn stable_subgraph_scores_preserve_geometry_and_every_poll() {
        struct Count {
            workspace: bool,
            owner: bool,
            polls: Cell<usize>,
            cancel: Option<usize>,
            panic: bool,
            #[cfg(feature = "diagnostic-traces")]
            mutate_owner: bool,
        }
        impl Context for Count {
            fn stable_sized_owner(&self) -> bool {
                self.owner
            }
            fn stable_compaction_workspace(&self) -> bool {
                self.workspace
            }
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                let count = self.polls.get() + 1;
                self.polls.set(count);
                if self.cancel.is_some_and(|cancel| count >= cancel) {
                    assert!(!self.panic, "subgraph scoring poll");
                    Some(ContextError::Cancelled)
                } else {
                    None
                }
            }
            #[cfg(feature = "diagnostic-traces")]
            fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
                let result = self.error();
                if self.mutate_owner && self.polls.get() == 3 {
                    owner.nodes[4].shape = ShapeKind::SqlTable;
                }
                result
            }
        }
        let mut input = Graph::default();
        for index in 0..5 {
            input.add_node(crate::Node::new(
                format!("n{index}"),
                Size {
                    width: 30.,
                    height: 20.,
                },
            ));
        }
        for (from, to) in [(0, 1), (0, 2), (1, 3)] {
            input.add_edge(crate::Edge {
                source: NodeId(from),
                target: NodeId(to),
            });
        }
        let nodes = [NodeId(0), NodeId(1)];
        let mut total_polls = 0;
        for cancel in std::iter::once(None).chain((1..).map(Some)) {
            if cancel.is_some_and(|cancel| cancel > total_polls) {
                break;
            }
            for panic in [false, true] {
                let mut outcomes = Vec::new();
                for (stable_workspace, stable_owner) in
                    [(false, false), (false, true), (true, false), (true, true)]
                {
                    let mut owner = ArenaGraph::from_input(&input);
                    for index in 0..5 {
                        owner.set_position(
                            NodeId(index),
                            Point {
                                x: index as f64 * 3.,
                                y: index as f64,
                            },
                        );
                    }
                    let mut workspace = owner.clone();
                    let mapping = CanonicalScoreMapping::new(
                        owner.topology_arena_id().unwrap(),
                        (0..5).map(NodeId).collect(),
                        (0..3).map(EdgeId).collect(),
                    );
                    let context = Count {
                        workspace: stable_workspace,
                        owner: stable_owner,
                        polls: Cell::new(0),
                        cancel,
                        panic,
                        #[cfg(feature = "diagnostic-traces")]
                        mutate_owner: false,
                    };
                    let mut session =
                        OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
                    let mut scorers =
                        Scorers::new(&nodes, context.stable_compaction_workspace(), &context);
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut scores = Vec::new();
                        for index in 0..3 {
                            // Both scored nodes, their adjacent node, and an
                            // unrelated node change geometry between scores.
                            for node in [NodeId(0), NodeId(1), NodeId(2), NodeId(4)] {
                                workspace.set_position(
                                    node,
                                    Point {
                                        x: (node.0 * 3 + index) as f64,
                                        y: (node.0 + index * 2) as f64,
                                    },
                                );
                            }
                            scores.push(
                                scorers
                                    .score(
                                        &mut workspace,
                                        &nodes,
                                        &mut Some(&mut session),
                                        &context,
                                    )
                                    .map_err(|error| error.to_string())?
                                    .to_bits(),
                            );
                        }
                        Ok::<_, String>(scores)
                    }))
                    .unwrap_or_else(|_| Err("subgraph scoring panic".into()));
                    drop(scorers);
                    drop(session);
                    let positions = (0..5)
                        .map(|index| {
                            let point = owner.position(NodeId(index)).unwrap();
                            (point.x.to_bits(), point.y.to_bits())
                        })
                        .collect::<Vec<_>>();
                    if cancel.is_none() {
                        let scores = result.as_ref().unwrap();
                        assert!(scores.windows(2).any(|pair| pair[0] != pair[1]));
                        total_polls = context.polls.get();
                    }
                    outcomes.push((result, positions, context.polls.get()));
                }
                assert_eq!(
                    outcomes[0], outcomes[1],
                    "fresh stable owner, cancel={cancel:?}, panic={panic}"
                );
                assert_eq!(
                    outcomes[0], outcomes[2],
                    "retained, cancel={cancel:?}, panic={panic}"
                );
                assert_eq!(
                    outcomes[0], outcomes[3],
                    "stable owner, cancel={cancel:?}, panic={panic}"
                );
            }
        }
        assert!(total_polls > 3);
        // The first score still admits unrelated carriers and ancestry. A
        // later scorer must also reject its own invalid local node after an
        // earlier scorer has successfully admitted the shared workspace.
        for fault in 0..4 {
            let scored = if fault == 3 {
                [nodes[0], NodeId(u32::MAX)]
            } else {
                nodes
            };
            let mut errors = Vec::new();
            for (stable_workspace, stable_owner) in
                [(false, false), (false, true), (true, false), (true, true)]
            {
                let mut owner = ArenaGraph::from_input(&input);
                for index in 0..5 {
                    owner.set_position(
                        NodeId(index),
                        Point {
                            x: index as f64 * 3.,
                            y: index as f64,
                        },
                    );
                }
                let mut workspace = owner.clone();
                match fault {
                    0 => owner.nodes[4].container = Some(NodeId(4)),
                    1 => workspace.nodes[4].tala_id += 1000,
                    2 => {
                        workspace.edge_records[2].source_arrow =
                            !workspace.edge_records[2].source_arrow;
                    }
                    _ => {}
                }
                let mapping = CanonicalScoreMapping::new(
                    owner.topology_arena_id().unwrap(),
                    (0..5).map(NodeId).collect(),
                    (0..3).map(EdgeId).collect(),
                );
                let context = Count {
                    workspace: stable_workspace,
                    owner: stable_owner,
                    polls: Cell::new(0),
                    cancel: None,
                    panic: false,
                    #[cfg(feature = "diagnostic-traces")]
                    mutate_owner: false,
                };
                let mut session =
                    OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
                let mut scorers =
                    Scorers::new(&scored, context.stable_compaction_workspace(), &context);
                let error = scorers
                    .score(&mut workspace, &scored, &mut Some(&mut session), &context)
                    .unwrap_err();
                errors.push((error.to_string(), context.polls.get()));
            }
            for error in &errors[1..] {
                assert_eq!(errors[0], *error, "initial admission fault {fault}");
            }
        }
        // Workspace stability alone does not admit owner reuse. A diagnostic
        // callback can change even an unrelated member after preparation has
        // captured it, and the next score must still reject that change.
        #[cfg(feature = "diagnostic-traces")]
        {
            let mut owner = ArenaGraph::from_input(&input);
            for index in 0..5 {
                owner.set_position(
                    NodeId(index),
                    Point {
                        x: index as f64 * 3.,
                        y: 0.,
                    },
                );
            }
            let mut workspace = owner.clone();
            let mapping = CanonicalScoreMapping::new(
                owner.topology_arena_id().unwrap(),
                (0..5).map(NodeId).collect(),
                (0..3).map(EdgeId).collect(),
            );
            let context = Count {
                workspace: true,
                owner: false,
                polls: Cell::new(0),
                cancel: None,
                panic: false,
                mutate_owner: true,
            };
            let mut session =
                OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
            let mut scorers = Scorers::new(&nodes, context.stable_compaction_workspace(), &context);
            scorers
                .score(&mut workspace, &nodes, &mut Some(&mut session), &context)
                .unwrap();
            let error = scorers
                .score(&mut workspace, &nodes, &mut Some(&mut session), &context)
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("topology changed during its movement sweep")
            );
        }
    }
}
