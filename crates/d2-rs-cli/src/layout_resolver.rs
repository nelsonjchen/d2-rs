// SPDX-License-Identifier: MPL-2.0
//! d2cli/layout.go: per-compilation, immutable built-in option snapshots.
use d2_rs_core::{Graph, LayoutEngineOptions, LayoutOptions, LayoutResult};
use d2_rs_layout::{EngineKind, LayoutDispatchError, LayoutDispatcher};
use d2_rs_tala::CancellationToken;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct LayoutResolver {
    bound: Arc<BoundOptions>,
}

struct BoundOptions {
    settings: LayoutEngineOptions,
    options: LayoutOptions,
}

impl LayoutResolver {
    pub(crate) fn new(settings: &LayoutEngineOptions, options: &LayoutOptions) -> Self {
        Self {
            bound: Arc::new(BoundOptions {
                settings: settings.clone(),
                options: options.clone(),
            }),
        }
    }

    pub(crate) fn resolve(&self, name: &str) -> Result<ResolvedLayout, String> {
        let engine = match name.to_ascii_lowercase().as_str() {
            "dagre" => EngineKind::Dagre,
            "elk" => EngineKind::Elk,
            "tala" => EngineKind::Tala,
            _ => return Err(super::layout_not_found(name)),
        };
        Ok(ResolvedLayout {
            bound: Arc::clone(&self.bound),
            engine,
        })
    }
}

pub(crate) struct ResolvedLayout {
    bound: Arc<BoundOptions>,
    engine: EngineKind,
}

impl ResolvedLayout {
    pub(crate) fn layout_compiled(
        &self,
        graph: &mut Graph,
        cancellation: Option<&CancellationToken>,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        graph.layout_engine_options = self.bound.settings.clone();
        LayoutDispatcher.layout_compiled_with_cancellation(
            graph,
            self.engine.clone(),
            &self.bound.options,
            cancellation,
        )
    }

    #[cfg(test)]
    pub(crate) fn layout(&self, graph: &mut Graph) -> Result<LayoutResult, LayoutDispatchError> {
        graph.layout_engine_options = self.bound.settings.clone();
        LayoutDispatcher.layout(graph, self.engine.clone(), &self.bound.options)
    }
}

#[cfg(test)]
#[path = "layout_resolver_api_tests.rs"]
mod api_tests;
