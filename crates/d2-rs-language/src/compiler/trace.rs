// SPDX-License-Identifier: MPL-2.0
//! Diagnostic-only key replay observations. This module and all call arguments
//! are removed unless compiler-trace is enabled.
use super::Compiler;
use d2_rs_core::{ast, ir::*};
use serde::Serialize;
#[derive(Debug, Serialize)]
pub struct KeyTrace {
    phase: &'static str,
    key_range: ast::Range,
    scope: String,
    target: Option<String>,
    lazy: bool,
    applying: bool,
    stack: usize,
    version: u64,
    queue: Vec<String>,
    post: Vec<String>,
    queued: Option<usize>,
}
impl Compiler {
    pub(super) fn trace_key(&mut self, phase: &'static str, context: &ast::Shared<RefContext>) {
        let name = |node| {
            let path: Vec<_> = crate::ir::ida(&self.arena, node)
                .into_iter()
                .flatten()
                .map(|p| ast::raw_string_box(ast::ScalarBox::from_child(p).scalar_bytes(), true))
                .collect();
            crate::format::format_key_path(&path)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        let context = context.read();
        let map = context.scope_map.unwrap();
        self.key_trace.push(KeyTrace {
            phase,
            key_range: context.key.as_ref().unwrap().read().range.clone(),
            scope: name(map),
            target: self.lazy_glob_target.map(name),
            lazy: self.lazy_glob_being_applied,
            applying: self.applying_lazy_worklist,
            stack: self.glob_ref_context_stack.len(),
            version: self.version(map),
            queue: self.lazy_glob_worklist.iter().map(|n| name(*n)).collect(),
            post: self.lazy_post_targets.iter().map(|n| name(*n)).collect(),
            queued: self.lazy_glob_queued.as_ref().map(|s| s.len()),
        });
    }
}
