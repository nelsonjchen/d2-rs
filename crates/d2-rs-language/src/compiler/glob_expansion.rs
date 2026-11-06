// SPDX-License-Identifier: MPL-2.0
//! d2ir/glob_expansion.go at master 6a7b48378: traversal and retained fields
//! have independent bounds; explicit source materialization is uncharged.
use super::{Compiler, edge_creation::error};
use d2_rs_core::{ast, ir::*};

pub(super) struct GlobExpansionBudget {
    pub limit: u64,
    used: u64,
    created_fields: u64,
}
impl Default for GlobExpansionBudget {
    fn default() -> Self {
        Self {
            limit: 65_536,
            used: 0,
            created_fields: 0,
        }
    }
}
impl GlobExpansionBudget {
    pub fn work_error(&self) -> String {
        format!("glob expansion exceeds limit of {} work units", self.limit)
    }
    pub fn reserve(&mut self, units: u64) -> Result<(), String> {
        if units > self.limit - self.used {
            return Err(self.work_error());
        }
        self.used += units;
        Ok(())
    }
    pub(super) fn reserve_field(&mut self) -> Result<(), String> {
        if self.created_fields >= 10_000 {
            return Err("glob expansion exceeds limit of 10000 created fields".into());
        }
        self.created_fields += 1;
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn usage(&self) -> (u64, u64) {
        (self.used, self.created_fields)
    }
}
impl Compiler {
    pub(super) fn glob_source(
        &self,
        context: Option<&ast::Shared<RefContext>>,
    ) -> Option<ast::Child> {
        context
            .and_then(|c| c.read().key.clone())
            .or_else(|| {
                self.glob_ref_context_stack
                    .last()
                    .and_then(|c| c.read().key.clone())
            })
            .map(ast::Child::Key)
    }
    pub(super) fn glob_failure(&mut self, source: Option<&ast::Child>, message: String) -> bool {
        if let Some(source) = source {
            self.errors.push(error(source, message));
        } else {
            self.errors.push(ast::Error {
                message,
                ..Default::default()
            });
        }
        self.halted = true;
        false
    }
    pub(super) fn reserve_glob_work(&mut self, source: Option<&ast::Child>, units: u64) -> bool {
        if self.stopped() {
            return false;
        }
        match self.glob_expansion.reserve(units) {
            Ok(()) => true,
            Err(message) => self.glob_failure(source, message),
        }
    }
    pub(super) fn reserve_glob_field(&mut self, source: Option<&ast::Child>) -> bool {
        if self.stopped() {
            return false;
        }
        match self.glob_expansion.reserve_field() {
            Ok(()) => true,
            Err(message) => self.glob_failure(source, message),
        }
    }
    pub(super) fn reserve_glob_generated_field_work(
        &mut self,
        mut parent: NodeId,
        source: Option<&ast::Child>,
    ) -> bool {
        if self.stopped() {
            return false;
        }
        let active = self.glob_ref_context_stack.len() as u64;
        if active == 0 {
            return true;
        }
        let mut depth = 1u64;
        while let Some(field) = self.arena.parent_field(parent) {
            if self.arena.parent(field).is_none() {
                break;
            }
            depth += 1;
            let Some(map) = self.arena.parent_map(field) else {
                break;
            };
            parent = map;
        }
        match depth.checked_mul(active) {
            Some(units) => self.reserve_glob_work(source, units),
            None => self.glob_failure(source, self.glob_expansion.work_error()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Original TestGlobExpansionBudgetDefault contract, including independent
    // retained-field exhaustion and refusal without incrementing either count.
    #[test]
    fn master_default_work_and_created_field_boundaries() {
        let mut budget = GlobExpansionBudget::default();
        assert_eq!(budget.limit, 65_536);
        for _ in 0..65_536 {
            budget.reserve(1).unwrap();
        }
        assert_eq!(
            budget.reserve(1).unwrap_err(),
            "glob expansion exceeds limit of 65536 work units"
        );
        assert_eq!(budget.used, 65_536);
        let mut budget = GlobExpansionBudget::default();
        for _ in 0..10_000 {
            budget.reserve_field().unwrap();
        }
        assert_eq!(
            budget.reserve_field().unwrap_err(),
            "glob expansion exceeds limit of 10000 created fields"
        );
        assert_eq!(budget.created_fields, 10_000);
        assert_eq!(budget.used, 0);
    }
}
