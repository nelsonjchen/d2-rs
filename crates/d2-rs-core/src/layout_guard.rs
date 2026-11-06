// SPDX-License-Identifier: MPL-2.0
//! Admission before the noninterruptible Dagre and ELK solvers.
//!
//! Source: D2 master 6a7b48378, d2layouts/internal/layoutguard/graph.go.
use crate::{CancellationToken, Graph, LayoutError, NodeId};
use std::collections::BTreeMap;

/// Checks the measured graph's raw inventories before geometry conversion.
pub fn check(
    engine: &str,
    graph: &Graph,
    cancellation: Option<&CancellationToken>,
) -> Result<(), LayoutError> {
    check_raw(
        engine,
        graph.nodes.len(),
        graph.edges.iter().map(|edge| {
            // Root endpoints have their own identity outside Graph::nodes.
            let exists = |id| id == NodeId::ROOT || graph.node(id).is_some();
            (exists(edge.source) && exists(edge.target)).then_some((edge.source, edge.target))
        }),
        cancellation,
    )
}

/// Checks raw endpoint identities, including absent edges or endpoints.
///
/// This projection retains the source guard's input contract without requiring
/// geometry or dereferencing an endpoint. `None` denotes an absent edge or at
/// least one absent endpoint; both have the same source admission error.
#[doc(hidden)]
pub fn check_raw(
    engine: &str,
    object_count: usize,
    edges: impl ExactSizeIterator<Item = Option<(NodeId, NodeId)>> + Clone,
    cancellation: Option<&CancellationToken>,
) -> Result<(), LayoutError> {
    let poll = || match cancellation.and_then(CancellationToken::error) {
        Some(error) => Err(error),
        None => Ok(()),
    };
    poll()?;
    let reject = |message| LayoutError::EngineValidation(format!("{engine} layout {message}"));
    if object_count > 1_024 {
        return Err(reject(format!(
            "object count {object_count} exceeds safe limit of 1024"
        )));
    }
    let edge_count = edges.len();
    if edge_count > 1_024 {
        return Err(reject(format!(
            "edge count {edge_count} exceeds safe limit of 1024"
        )));
    }
    let mut incident = BTreeMap::<NodeId, usize>::new();
    for (i, edge) in edges.clone().enumerate() {
        if i % 64 == 0 {
            poll()?;
        }
        let Some((source, target)) = edge else {
            return Err(reject("graph contains an edge without endpoints".into()));
        };
        *incident.entry(source).or_default() += 1;
        *incident.entry(target).or_default() += 1;
    }
    if edge_count <= 256 {
        return Ok(());
    }
    let mut work = 0;
    for (i, edge) in edges.enumerate() {
        if i % 64 == 0 {
            poll()?;
        }
        let (source, target) = edge.expect("validated endpoint presence");
        let units = incident[&source].min(incident[&target]);
        if units > 8_192 - work {
            return Err(reject(
                "graph is too interconnected: edge interaction work exceeds safe limit of 8192"
                    .into(),
            ));
        }
        work += units;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_precedes_raw_limits_and_malformed_edges() {
        for expired in [false, true] {
            let token = CancellationToken::new();
            if expired {
                token.expire();
            } else {
                token.cancel();
            }
            let error = check_raw("dagre", 1_025, [None].into_iter(), Some(&token)).unwrap_err();
            assert_eq!(
                error.to_string(),
                if expired {
                    "context deadline exceeded"
                } else {
                    "context canceled"
                }
            );
        }
    }

    #[test]
    fn raw_limits_and_endpoint_presence_precede_small_graph_fast_path() {
        assert_eq!(
            check_raw("ELK", 1_025, [].into_iter(), None)
                .unwrap_err()
                .to_string(),
            "ELK layout object count 1025 exceeds safe limit of 1024"
        );
        let malformed = [None];
        assert_eq!(
            check_raw("ELK", 0, malformed.into_iter(), None)
                .unwrap_err()
                .to_string(),
            "ELK layout graph contains an edge without endpoints"
        );
        let repeated = vec![Some((NodeId(0), NodeId(1))); 1_025];
        assert_eq!(
            check_raw("ELK", 2, repeated.into_iter(), None)
                .unwrap_err()
                .to_string(),
            "ELK layout edge count 1025 exceeds safe limit of 1024"
        );
    }

    #[test]
    fn sparse_graphs_survive_and_unrelated_objects_do_not_dilute_density() {
        let star = (1..1_024)
            .map(|i| Some((NodeId(0), NodeId(i))))
            .collect::<Vec<_>>();
        check_raw("test", 1_024, star.into_iter(), None).unwrap();
        let dense = (0..20)
            .flat_map(|s| {
                (0..20)
                    .filter(move |t| *t != s)
                    .map(move |t| Some((NodeId(s), NodeId(t))))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            check_raw("test", 1_020, dense.into_iter(), None)
                .unwrap_err()
                .to_string(),
            "test layout graph is too interconnected: edge interaction work exceeds safe limit of 8192"
        );
        check_raw(
            "test",
            2,
            vec![Some((NodeId(0), NodeId(1))); 256].into_iter(),
            None,
        )
        .unwrap();
    }
}
