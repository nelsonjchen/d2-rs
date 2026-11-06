// SPDX-License-Identifier: MPL-2.0
//! Pinned d2scenebuild legendConnectionIconTarget/buildLegendConnectionIcon.
use super::*;
use d2_rs_core::LegendConnection;

pub(super) fn validate(legend: Option<&Legend>) -> Result<(), RenderError> {
    let Some(legend) = legend else { return Ok(()) };
    for (index, c) in legend
        .connections
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.label.is_empty())
    {
        let reason = if c.has_icon {
            Some(
                "connection icon asset (svg legend connections render only route and arrow styles)",
            )
        } else if !c.source_label.is_empty() || !c.target_label.is_empty() {
            Some("endpoint label (svg legend connections render only route and arrow styles)")
        } else if c.link.as_deref().is_some_and(|s| !s.is_empty())
            || c.tooltip.as_deref().is_some_and(|s| !s.is_empty())
        {
            Some("link/tooltip metadata on a legend connection")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(RenderError::LegendMetadata {
                object: format!(
                    "legend connection[{index}] {}",
                    d2_rs_core::quote_go_bytes(c.source_id.as_bytes())
                ),
                reason,
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    svg: &mut String,
    c: &LegendConnection,
    index: usize,
    x: f64,
    y: f64,
    palette: PaletteView<'_>,
    precision: bool,
    mut sources: Option<&mut Vec<RasterPathSource>>,
) {
    // d2exporter.toConnection applies GetStroke and host C4 defaults before
    // legendConnectionIconTarget copies the target's route paint fields.
    let target_style = themed_edge_style(
        EdgeStyle {
            stroke: c.stroke.clone(),
            stroke_dash: c.stroke_dash.clone(),
            ..Default::default()
        },
        palette,
    );
    let mut target = c.clone();
    target.stroke = target_style.stroke;
    target.stroke_dash = target_style.stroke_dash;
    let c = &target;
    let points = [Point { x: 0.0, y: 0.0 }, Point { x: 48.0, y: 0.0 }];
    let width = stroke_width(c.stroke_width.as_deref());
    let source_adjustment = arrowhead_adjustment(points[1], points[0], c.source_arrow, width, 0.0);
    let target_adjustment = arrowhead_adjustment(points[0], points[1], c.target_arrow, width, 0.0);
    let start = Point {
        x: points[0].x + source_adjustment.x,
        y: 0.0,
    };
    let end = Point {
        x: points[1].x + target_adjustment.x,
        y: 0.0,
    };
    let stroke = themed_color(
        c.stroke.as_deref(),
        &palette.color("B1", palette.base.b1),
        palette,
    );
    let opacity = c
        .opacity
        .clone()
        .map(render_opacity_value)
        .unwrap_or_else(|| "1".into());
    let _ = writeln!(
        svg,
        "<g transform=\"translate({} {}) scale(0.5)\"><g opacity=\"{}\">",
        number(x),
        number(y),
        escape(&opacity)
    );
    let effective_dash = if c.animated
        && c.stroke_dash
            .as_deref()
            .and_then(d2_rs_core::parse_d2_float)
            .unwrap_or(0.0)
            == 0.0
    {
        Some("5")
    } else {
        c.stroke_dash.as_deref()
    };
    let dash = stroke_dasharray(c.stroke_width.as_deref().unwrap_or("2"), effective_dash)
        .map(|s| format!(" stroke-dasharray=\"{}\"", escape(&s)))
        .unwrap_or_default();
    let native_dash = if precision {
        stroke_dash_values(c.stroke_width.as_deref().unwrap_or("2"), effective_dash)
            .map(|(d, g)| raster_dash_attributes(d, g))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let timing = if c.animated && width > 0.0 {
        stroke_dash_values(c.stroke_width.as_deref().unwrap_or("2"), effective_dash)
    } else {
        None
    };
    let arrow = |target: bool| {
        let enabled = if target {
            c.target_arrow
        } else {
            c.source_arrow
        };
        enabled
            .then(|| {
                raster_inline_arrowhead(
                    if target {
                        c.arrowheads.target.as_deref()
                    } else {
                        c.arrowheads.source.as_deref()
                    },
                    format!(
                        "d2-rs-native-legend-connection-{index}-arrow-{}",
                        if target { "dst" } else { "src" }
                    ),
                    &points,
                    &points,
                    target,
                    if target {
                        target_adjustment
                    } else {
                        source_adjustment
                    },
                    0.0,
                    0.0,
                    width,
                    &stroke,
                    &palette.color("N7", palette.base.neutrals.n7),
                )
            })
            .flatten()
    };
    let source_arrow = arrow(false);
    let target_arrow = arrow(true);
    let emit_arrow = |svg: &mut String,
                      a: Option<&RasterInlineArrow>,
                      sources: &mut Option<&mut Vec<RasterPathSource>>| {
        if let Some(a) = a {
            for path in &a.paths {
                if let Some(sources) = sources.as_deref_mut() {
                    sources.push(RasterPathSource {
                        id: path.id.clone(),
                        commands: path.commands.clone(),
                        transform: path.source_transform,
                    });
                }
                let outline = path
                    .stroke
                    .as_ref()
                    .map(|s| {
                        format!(
                            " stroke=\"{}\" stroke-width=\"{}\"",
                            escape(s),
                            number(path.stroke_width.unwrap_or(width))
                        )
                    })
                    .unwrap_or_default();
                let style = path
                    .stroke_style
                    .map(|s| format!(" style=\"{s}\""))
                    .unwrap_or_default();
                let _ = writeln!(
                    svg,
                    "<path id=\"{}\" d=\"{}\" fill=\"{}\"{outline}{style}/>",
                    escape(&path.id),
                    path.path_data,
                    escape(&path.fill)
                );
            }
        }
    };
    let split = c.animated && c.source_arrow == c.target_arrow;
    let middle = Point {
        x: (start.x + end.x) * 0.5,
        y: 0.0,
    };
    let segments = if split {
        vec![(start, middle, true), (middle, end, false)]
    } else {
        vec![(start, end, false)]
    };
    for (part, (from, to, reverse)) in segments.into_iter().enumerate() {
        let animation = timing
            .map(|(d, g)| {
                let direction = if c.source_arrow && !c.target_arrow {
                    10.0
                } else {
                    -10.0
                };
                let offset = direction * (d + g);
                format!(
                    " stroke-dashoffset=\"{}\" animation=\"dashdraw {}s linear infinite{}\"",
                    number(offset),
                    number(g * 0.5),
                    if reverse { " reverse" } else { "" }
                )
            })
            .unwrap_or_default();
        let id = format!("d2-rs-native-legend-connection-{index}-route-{part}");
        if let Some(sources) = sources.as_deref_mut() {
            sources.push(RasterPathSource {
                id: id.clone(),
                commands: vec![
                    RasterPathCommand::MoveTo(from),
                    RasterPathCommand::LineTo(to),
                ],
                transform: None,
            });
        }
        let _ = writeln!(
            svg,
            "<path id=\"{id}\" d=\"M {} {} L {} {}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{dash}{native_dash}{animation}/>",
            number(from.x),
            number(from.y),
            number(to.x),
            number(to.y),
            escape(&stroke),
            number(width)
        );
        if part == 0 {
            emit_arrow(svg, source_arrow.as_ref(), &mut sources);
        }
        if !split || part == 1 {
            emit_arrow(svg, target_arrow.as_ref(), &mut sources);
        }
    }
    svg.push_str("</g></g>\n");
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod source_contract {
    use super::*;
    #[test]
    fn original_go_connection_icon_routes_strokes_and_arrow_inventory() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/legend-connections-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 384);
        for row in rows.as_array().unwrap() {
            let c = &row["connection"];
            let src = c["srcArrow"].as_str().unwrap();
            let dst = c["dstArrow"].as_str().unwrap();
            let connection = LegendConnection {
                label: "edge".into(),
                source_arrow: src != "none",
                target_arrow: dst != "none",
                arrowheads: d2_rs_core::EdgeArrowheads {
                    source: Some(src.into()),
                    target: Some(dst.into()),
                },
                stroke: Some("B1".into()),
                stroke_width: Some(c["strokeWidth"].to_string()),
                stroke_dash: Some(c["strokeDash"].to_string()),
                animated: c["animated"].as_bool().unwrap(),
                ..Default::default()
            };
            let nodes = row["nodes"].as_array().unwrap();
            let transform = &nodes[0]["transform"];
            let x = transform["E"].as_f64().unwrap();
            let y = transform["F"].as_f64().unwrap();
            assert_eq!(
                (
                    transform["A"].as_f64().unwrap(),
                    transform["D"].as_f64().unwrap()
                ),
                (0.5, 0.5)
            );
            let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\">");
            let mut sources = Vec::new();
            draw(
                &mut svg,
                &connection,
                0,
                x,
                y,
                PaletteView::new(active_palette(None), &BTreeMap::new()),
                true,
                Some(&mut sources),
            );
            svg.push_str("</svg>");
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let actual = doc
                .descendants()
                .filter(|n| {
                    n.has_tag_name("path") && n.attribute("stroke-linecap") == Some("round")
                })
                .collect::<Vec<_>>();
            let expected = nodes
                .iter()
                .filter(|n| {
                    n["id"]
                        .as_str()
                        .unwrap()
                        .starts_with("legend:connection:0:icon:path")
                })
                .collect::<Vec<_>>();
            assert_eq!(actual.len(), expected.len(), "{}", row["name"]);
            for (a, e) in actual.iter().zip(expected) {
                let p = &e["primitive"];
                let commands = p["Commands"].as_array().unwrap();
                assert_eq!(commands.len(), 2);
                let data = format!(
                    "M {} {} L {} {}",
                    number(commands[0]["P1"]["X"].as_f64().unwrap()),
                    number(commands[0]["P1"]["Y"].as_f64().unwrap()),
                    number(commands[1]["P1"]["X"].as_f64().unwrap()),
                    number(commands[1]["P1"]["Y"].as_f64().unwrap())
                );
                assert_eq!(a.attribute("d").unwrap(), data, "{}", row["name"]);
                if !p["Stroke"].is_null() {
                    assert_eq!(
                        a.attribute("stroke-width").unwrap().parse::<f64>().unwrap(),
                        p["Stroke"]["Width"].as_f64().unwrap()
                    );
                    assert_eq!(p["Stroke"]["Cap"], 1);
                    assert_eq!(p["Stroke"]["Join"], 1);
                    if let Some(d) = p["Stroke"]["Dashes"].as_array() {
                        for (name, value) in [
                            ("data-d2-rs-dash-size-bits", &d[0]),
                            ("data-d2-rs-dash-gap-bits", &d[1]),
                        ] {
                            let bits = u64::from_str_radix(a.attribute(name).unwrap(), 16).unwrap();
                            assert_eq!(
                                f64::from_bits(bits),
                                value.as_f64().unwrap(),
                                "{} {name}",
                                row["name"]
                            );
                        }
                    }
                }
            }
            let source_leaves = nodes
                .iter()
                .filter(|n| {
                    n["id"]
                        .as_str()
                        .unwrap()
                        .starts_with("legend:connection:0:icon")
                        && !n["primitive"].is_null()
                })
                .count();
            assert_eq!(
                doc.descendants().filter(|n| n.has_tag_name("path")).count(),
                source_leaves,
                "{} paint leaf inventory",
                row["name"]
            );
            assert!(!svg.contains("url(#arrow)"));
        }
    }
}

#[cfg(test)]
mod admission_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_legend_connection_metadata_rejections_and_empty_labels() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/legend-connection-errors-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 9);
        for row in rows.as_array().unwrap() {
            let parsed =
                d2_rs_language::parse("input.d2", row["source"].as_str().unwrap().as_bytes(), None);
            assert!(parsed.error.is_none());
            let (compiler, root) =
                d2_rs_language::compiler::compile_ir(d2_rs_core::ast::Shared::new(parsed.map))
                    .unwrap();
            let graph = d2_rs_language::compiler::lower_graph(&compiler, root).unwrap();
            let actual = validate(graph.legend.as_ref());
            if row["exit"] == 0 {
                assert!(actual.is_ok(), "{}", row["name"]);
            } else {
                let expected = format!(
                    "scene: {}",
                    row["stderr"]
                        .as_str()
                        .unwrap()
                        .split_once("scene: ")
                        .unwrap()
                        .1
                        .trim_end()
                );
                assert_eq!(actual.unwrap_err().to_string(), expected, "{}", row["name"]);
            }
        }
    }
}
