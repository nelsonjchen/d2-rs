// SPDX-License-Identifier: MPL-2.0
//! d2ir/edge_expansion.go: distinct endpoint pairs and repeated examinations
//! have separate compilation-wide budgets. Explicit endpoints are uncharged.
use super::{Compiler, edge_creation::error};
use d2_rs_core::{ast, ir::*};
use std::collections::HashSet;

pub(super) struct EdgeExpansion {
    pub limit: u64,
    pub work_limit: u64,
    used: u64,
    work_used: u64,
    pairs: HashSet<(Option<GlobId>, usize, NodeId, NodeId, bool)>,
}
impl Default for EdgeExpansion {
    fn default() -> Self {
        Self {
            limit: 1_024,
            work_limit: 65_536,
            used: 0,
            work_used: 0,
            pairs: HashSet::new(),
        }
    }
}
impl EdgeExpansion {
    pub(super) fn reserve_pair(&mut self) -> Result<(), String> {
        if self.used >= self.limit {
            return Err(format!(
                "edge glob expansion exceeds limit of {} endpoint pairs",
                self.limit
            ));
        }
        self.used += 1;
        Ok(())
    }
    pub(super) fn reserve_work(&mut self) -> Result<(), String> {
        if self.work_used >= self.work_limit {
            return Err(format!(
                "edge glob expansion exceeds work limit of {} endpoint-pair examinations",
                self.work_limit
            ));
        }
        self.work_used += 1;
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn usage(&self) -> (u64, u64) {
        (self.used, self.work_used)
    }
}

impl Compiler {
    pub(super) fn reserve_edge_expansion(
        &mut self,
        segment: &ast::Shared<ast::Edge>,
        glob: Option<GlobId>,
        src: NodeId,
        dst: NodeId,
        selector: bool,
    ) -> bool {
        if self.stopped() {
            return false;
        }
        let budget = &mut self.edge_expansion;
        let reservation = budget.reserve_work().and_then(|()| {
            let pair = (
                glob,
                &*segment.read() as *const _ as usize,
                src,
                dst,
                selector,
            );
            if budget.pairs.contains(&pair) {
                return Ok(());
            }
            budget.reserve_pair()?;
            budget.pairs.insert(pair);
            Ok(())
        });
        if let Err(message) = reservation {
            self.errors
                .push(error(&ast::Child::Edge(segment.clone()), message));
            self.halted = true;
            return false;
        }
        true
    }
}
