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
    pub(super) fn new(nodes: &[NodeId], stable: bool) -> Self {
        Self(stable.then(|| {
            nodes
                .iter()
                .map(|&n| OwnerSizelessEdgeLengthScorer::new(n, true))
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
        for scorer in scorers {
            total += session
                .score_sizeless_owner(scorer, graph, score_context)
                .map_err(compaction_candidates::optimization_error)?;
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
