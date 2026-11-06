// SPDX-License-Identifier: MPL-2.0
// Source code token accounting follows d2scenebuild buildDocument object order.
use super::*;
use d2_rs_text::code::{self, CodeBudget, CodeWorkError};
#[derive(Default)]
pub(super) struct Streams {
    pub nodes: BTreeMap<NodeId, Option<code_shape::Runs>>,
    pub edges: BTreeMap<EdgeId, Option<code_shape::Runs>>,
}
pub(super) fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), RenderError> {
    if let Some(error) = cancel.and_then(d2_rs_core::CancellationToken::error) {
        return Err(RenderError::CodeContext {
            deadline: matches!(error, d2_rs_core::LayoutError::DeadlineExceeded),
        });
    }
    Ok(())
}
pub(super) fn collect(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    budget: &mut CodeBudget,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Streams, RenderError> {
    let mut streams = Streams::default();
    let mut objects = Vec::new();
    for id in render_node_ids(graph) {
        objects.push((
            sequence_shape_z_index(graph, id),
            Some(node_level(graph, id)),
            true,
            id.0,
        ));
    }
    for (id, edge) in graph.edges() {
        objects.push((sequence_connection_z_index(graph, edge), None, false, id.0));
    }
    objects.sort_by(|a, b| {
        a.0.cmp(&b.0).then_with(|| match (a.1, b.1) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        })
    });
    let palette = PaletteView::new(active_palette(options.theme_id), &options.theme_overrides);
    for (_, _, shape, id) in objects {
        context(cancel)?;
        let (object, language, label) = if shape {
            let id = NodeId(id);
            let node = graph.node(id).unwrap();
            let base = styles.get(&id).cloned().unwrap_or_default();
            if base.opacity.as_deref().and_then(d2_rs_core::parse_d2_float) == Some(0.0)
                || is_sequence_span_like(graph, id)
            {
                continue;
            }
            let style = themed_node_style(graph, id, &base, palette);
            let label = labels.get(&id).map_or("", String::as_str);
            (
                format!(
                    "shape {}",
                    d2_rs_core::quote_go_bytes(node.external_id.as_bytes())
                ),
                style.language.clone().unwrap_or_default(),
                theme_label(
                    label,
                    Some(&style),
                    target_shape_for_node(graph, id).unwrap_or(node.shape),
                    palette,
                ),
            )
        } else {
            let id = EdgeId(id);
            let style = graph.edge_style(id);
            let effective = effective_edge_label(graph, result, id);
            let label = effective.as_ref().map_or("", |label| label.text.as_str());
            // assets.go skips these during preflight, but connection.go still
            // builds their labels in stable object order, even at opacity zero.
            if !label.is_empty()
                && style.language.as_deref() == Some("latex")
                && style
                    .opacity
                    .as_deref()
                    .and_then(d2_rs_core::parse_d2_float)
                    == Some(0.0)
            {
                let object = format!(
                    "connection {}",
                    d2_rs_core::quote_go_bytes(
                        edge_svg_id(graph, id).unwrap_or_default().as_bytes()
                    )
                );
                let math = d2_rs_math::render(label).map_err(|error| RenderError::LatexScene {
                    object: object.clone(),
                    stage: "render",
                    error: error.to_string(),
                })?;
                context(cancel)?;
                d2_rs_math::import_scene_svg(&math).map_err(|error| RenderError::LatexScene {
                    object,
                    stage: "SVG",
                    error,
                })?;
                context(cancel)?;
            }
            (
                format!(
                    "connection {}",
                    d2_rs_core::quote_go_bytes(
                        edge_svg_id(graph, id).unwrap_or_default().as_bytes()
                    )
                ),
                style.language.clone().unwrap_or_default(),
                theme_edge_label(label, &style, palette),
            )
        };
        if label.is_empty()
            || language.is_empty()
            || matches!(language.as_str(), "markdown" | "latex")
        {
            continue;
        }
        let wrap = |error: CodeWorkError| RenderError::CodeWork {
            object: object.clone(),
            language: language.clone(),
            error,
        };
        let mut tokens = code::tokenize_bounded(&language, &label, budget, cancel).map_err(wrap)?;
        if tokens.is_none() && !code::registered(&language) {
            tokens = Some(code::plain_bounded(&label, budget, cancel).map_err(wrap)?);
        }
        let font_size = if shape {
            let node = graph.node(NodeId(id)).unwrap();
            result
                .node_labels
                .get(&NodeId(id))
                .and_then(|s| s.font_size)
                .or(node.font_size)
                .unwrap_or(16)
        } else {
            graph.edge_font_size(EdgeId(id))
        };
        let dark = options
            .theme_id
            .and_then(theme::find)
            .is_some_and(|t| t.dark);
        let runs = tokens
            .map(|tokens| code_shape::prepare(&object, tokens, font_size, dark, cancel))
            .transpose()?;
        if shape {
            streams.nodes.insert(NodeId(id), runs);
        } else {
            streams.edges.insert(EdgeId(id), runs);
        }
    }
    Ok(streams)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_rs_tala::{Engine, LayoutOptions};
    #[test]
    fn math_preflight_uses_emitted_layout_label() {
        for opacity in ["0", "1"] {
            let mut graph = Graph::default();
            let a = graph.add_node(Node::new(
                "a",
                Size {
                    width: 100.,
                    height: 100.,
                },
            ));
            let b = graph.add_node(Node::new(
                "b",
                Size {
                    width: 100.,
                    height: 100.,
                },
            ));
            let edge = graph.add_edge(d2_rs_core::Edge {
                source: a,
                target: b,
            });
            let label = d2_rs_core::EdgeLabel {
                text: "x".into(),
                size: Size {
                    width: 20.,
                    height: 20.,
                },
                position: LabelPosition::InsideMiddleCenter,
                percentage: 0.5,
            };
            graph.set_edge_label(edge, Some(label.clone()));
            graph.set_edge_style(
                edge,
                EdgeStyle {
                    language: Some("latex".into()),
                    opacity: Some(opacity.into()),
                    ..Default::default()
                },
            );
            let mut result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
            result.edge_labels.insert(edge, label);
            graph.edge_label(edge).unwrap();
            let mut stale = graph.edge_label(edge).unwrap().clone();
            stale.text = "\\badcommand".into();
            graph.set_edge_label(edge, Some(stale));
            assert!(
                render_svg_with_raster_sources_checked(
                    &graph,
                    &result,
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                    RenderOptions::default(),
                    false
                )
                .is_ok()
            );
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_hidden_math_and_code_error_order() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/hidden-math-order-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let mut graph = Graph::default();
            let mut node = Node::new(
                "a",
                Size {
                    width: 100.,
                    height: 100.,
                },
            );
            node.shape = ShapeKind::Code;
            node.font_size = Some(16);
            let a = graph.add_node(node);
            let b = graph.add_node(Node::new(
                "b",
                Size {
                    width: 100.,
                    height: 100.,
                },
            ));
            let edge = graph.add_edge(d2_rs_core::Edge {
                source: a,
                target: b,
            });
            graph.set_edge_label(
                edge,
                Some(d2_rs_core::EdgeLabel {
                    text: row["formula"].as_str().unwrap().into(),
                    size: Size {
                        width: 20.,
                        height: 20.,
                    },
                    position: LabelPosition::InsideMiddleCenter,
                    percentage: 0.5,
                }),
            );
            graph.set_edge_style(
                edge,
                EdgeStyle {
                    language: Some("latex".into()),
                    opacity: Some(row["opacity"].as_u64().unwrap().to_string()),
                    ..Default::default()
                },
            );
            let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
            let labels = BTreeMap::from([(a, row["code"].as_str().unwrap().to_owned())]);
            let styles = BTreeMap::from([(
                a,
                SourceNodeStyle {
                    language: Some("MD".into()),
                    ..Default::default()
                },
            )]);
            let actual = render_svg_with_raster_sources_checked(
                &graph,
                &result,
                &labels,
                &styles,
                RenderOptions::default(),
                false,
            )
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
            assert_eq!(actual, row["error"].as_str().unwrap(), "{row}");
        }
    }
    #[test]
    fn glyph_failure_precedes_later_owner_budget_and_retains_original_byte_offsets() {
        let mut graph = Graph::default();
        let mut labels = BTreeMap::new();
        let mut styles = BTreeMap::new();
        for (name, label) in [("a", "🙂".to_owned()), ("b", "a".repeat(100000))] {
            let mut node = Node::new(
                name,
                Size {
                    width: 100.0,
                    height: 100.0,
                },
            );
            node.shape = ShapeKind::Code;
            node.font_size = Some(16);
            let id = graph.add_node(node);
            labels.insert(id, label);
            styles.insert(
                id,
                SourceNodeStyle {
                    language: Some("MD".into()),
                    ..Default::default()
                },
            );
        }
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let mut budget = CodeBudget::default();
        assert_eq!(
            render_svg_with_raster_sources_controlled(
                &graph,
                &result,
                &labels,
                &styles,
                RenderOptions::default(),
                false,
                &mut budget,
                None
            )
            .unwrap_err()
            .to_string(),
            "scene: shape \"a\" code token 0:0: missing glyph U+1F642 at byte 0"
        );
        assert_eq!(budget.counts(), (1, 1));
        assert!(
            render_svg_with_sketch_checked(
                &graph,
                &result,
                &labels,
                &styles,
                RenderOptions::default(),
                false
            )
            .is_ok(),
            "public SVG keeps browser font fallback"
        );
        labels.insert(NodeId(0), "a\t🙂".to_owned());
        labels.insert(NodeId(1), String::new());
        styles.get_mut(&NodeId(0)).unwrap().language = Some("boguslanguage".into());
        let mut budget = CodeBudget::default();
        assert_eq!(
            render_svg_with_raster_sources_controlled(
                &graph,
                &result,
                &labels,
                &styles,
                RenderOptions::default(),
                false,
                &mut budget,
                None
            )
            .unwrap_err()
            .to_string(),
            "scene: shape \"a\" code token 0:0: missing glyph U+1F642 at byte 5"
        );
    }

    #[test]
    fn default_document_ceiling_is_reached_after_valid_geometry_without_partial_svg() {
        let mut graph = Graph::default();
        let mut labels = BTreeMap::new();
        let mut styles = BTreeMap::new();
        for (name, label) in [("a", "a".repeat(100000)), ("b", "b".to_owned())] {
            let mut node = Node::new(
                name,
                Size {
                    width: 100.0,
                    height: 100.0,
                },
            );
            node.shape = ShapeKind::Code;
            node.font_size = Some(16);
            let id = graph.add_node(node);
            labels.insert(id, label);
            styles.insert(
                id,
                SourceNodeStyle {
                    language: Some("MD".into()),
                    ..Default::default()
                },
            );
        }
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let mut budget = CodeBudget::default();
        assert_eq!(
            render_svg_with_raster_sources_controlled(
                &graph,
                &result,
                &labels,
                &styles,
                RenderOptions::default(),
                false,
                &mut budget,
                None
            )
            .unwrap_err()
            .to_string(),
            "scene: shape \"b\" tokenize MD code: raw lexer token count exceeds limit 100000"
        );
        assert_eq!(budget.counts(), (100000, 100000));
        for style in styles.values_mut() {
            style.opacity = Some("0".into());
        }
        let mut budget = CodeBudget::default();
        let (svg, _) = render_svg_with_raster_sources_controlled(
            &graph,
            &result,
            &labels,
            &styles,
            RenderOptions::default(),
            false,
            &mut budget,
            None,
        )
        .unwrap();
        assert_eq!(budget.counts(), (0, 0));
        let doc = roxmltree::Document::parse(std::str::from_utf8(&svg).unwrap()).unwrap();
        assert_eq!(
            doc.descendants().filter(|n| n.has_tag_name("text")).count(),
            0
        );
    }

    #[test]
    fn native_code_cache_shares_budget_and_preserves_pre_cancel_cause() {
        let mut graph = Graph::default();
        let a = graph.add_node(Node::new(
            "a",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let b = graph.add_node(Node::new(
            "b",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let labels = BTreeMap::from([(a, "x".into()), (b, "y".into())]);
        let styles = BTreeMap::from([
            (
                a,
                SourceNodeStyle {
                    language: Some("go".into()),
                    ..Default::default()
                },
            ),
            (
                b,
                SourceNodeStyle {
                    language: Some("go".into()),
                    ..Default::default()
                },
            ),
        ]);
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let mut budget = CodeBudget::for_max_nodes(1);
        assert_eq!(
            render_svg_with_raster_sources_controlled(
                &graph,
                &result,
                &labels,
                &styles,
                RenderOptions::default(),
                false,
                &mut budget,
                None
            )
            .unwrap_err()
            .to_string(),
            "scene: shape \"b\" tokenize go code: raw lexer token count exceeds limit 1"
        );
        assert_eq!(budget.counts(), (1, 1));
        for deadline in [false, true] {
            let token = d2_rs_core::CancellationToken::new();
            if deadline {
                token.expire();
            } else {
                token.cancel();
            }
            let mut budget = CodeBudget::default();
            assert_eq!(
                render_svg_with_raster_sources_controlled(
                    &graph,
                    &result,
                    &labels,
                    &styles,
                    RenderOptions::default(),
                    false,
                    &mut budget,
                    Some(&token)
                )
                .unwrap_err()
                .to_string(),
                if deadline {
                    "context deadline exceeded"
                } else {
                    "context canceled"
                }
            );
            assert_eq!(budget.counts(), (0, 0));
        }
    }
}
