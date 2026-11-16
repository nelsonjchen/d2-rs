// SPDX-License-Identifier: MPL-2.0
//! Input checks for one LayoutNested core-layout scope, without laying it
//! out. Used to replay where d2lib.Compile stops when a core layout fails.
use d2_rs_core::{Graph, NodeId};

/// Whether TALA rejects the scope graph made of `live` objects (the objects
/// of a LayoutNested graph when coreLayout runs) before placement: the seed
/// plan and validateD2Objects of d2talalayout (layout.go:60-100,
/// adapter_validate.go:100-260), on the same projection the dispatcher
/// lays out.
pub fn validate_tala_scope(
    graph: &Graph,
    live: &[NodeId],
    options: &d2_rs_tala::LayoutOptions,
) -> Result<(), String> {
    let mut visible = live.to_vec();
    visible.sort();
    visible.dedup();
    let (mut projected, nodes, _) = super::project_graph(graph, &visible, false);
    // A container whose contents LayoutNested extracted and laid out first
    // is fitted to them (FitToGraph) before this core layout. Its size is at
    // least its largest descendant's, which is what the coordinate limits
    // compare.
    for (index, id) in nodes.iter().enumerate() {
        let mut width = 0.0_f64;
        let mut height = 0.0_f64;
        for (candidate, node) in graph.nodes() {
            if candidate != *id
                && !visible.contains(&candidate)
                && std::iter::successors(node.parent, |parent| graph.node(*parent)?.parent)
                    .any(|ancestor| ancestor == *id)
            {
                width = width.max(node.size.width);
                height = height.max(node.size.height);
            }
        }
        if let Some(node) = projected.nodes.get_mut(index) {
            node.size.width = node.size.width.max(width);
            node.size.height = node.size.height.max(height);
        }
    }
    d2_rs_tala::Engine
        .validate_layout_input(&projected, options)
        .map_err(|error| error.to_string())
}

/// Whether an ELK core layout of a nonempty graph fails on the configured
/// algorithm name (elk-go's algorithm lookup; empty keeps the default).
pub fn validate_elk_algorithm(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Ok(());
    }
    d2_rs_elk::ElkAlgorithm::parse(name)
        .map(|_| ())
        .map_err(|error| error.to_string())
}
