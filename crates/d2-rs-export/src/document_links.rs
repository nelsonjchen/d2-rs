// SPDX-License-Identifier: MPL-2.0
//! D2 v0.9.0 d2scenebuild/metadata.go target-space hit rectangles.
use super::*;
use d2_rs_core::{LabelPosition, Size};
use serde_json::Value;

fn number(value: &Value, field: &str) -> f64 {
    value[field].as_f64().unwrap_or(0.0)
}

pub(super) fn target_regions(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> Result<Vec<DocumentLinkMetadata>, ExportError> {
    let shapes = d2_rs_render::target_shapes_metadata(graph, result, labels, styles)
        .map_err(|e| ExportError::Pdf(e.to_string()))?;
    let connections = d2_rs_render::target_connections_metadata(graph, result, styles)
        .map_err(|e| ExportError::Pdf(e.to_string()))?;
    let mut regions = Vec::new();
    // Target shape order, integer publication and full stroke expansion all
    // precede the viewport mapping in the original metadata builder.
    for shape in shapes {
        let href = shape["link"].as_str().unwrap_or_default();
        if href.is_empty() && shape["tooltip"].as_str().unwrap_or_default().is_empty() {
            continue;
        }
        let stroke = number(&shape, "strokeWidth");
        regions.push(DocumentLinkMetadata {
            tooltip: shape["tooltip"].as_str().unwrap_or_default().to_owned(),
            board_target: is_board_target(href),
            region: DocumentLinkRegion {
                href: href.to_owned(),
                rect: Rect {
                    origin: Point {
                        x: number(&shape["pos"], "x") - stroke,
                        y: number(&shape["pos"], "y") - stroke,
                    },
                    size: Size {
                        width: number(&shape, "width") + 2.0 * stroke,
                        height: number(&shape, "height") + 2.0 * stroke,
                    },
                },
            },
        });
    }
    for connection in connections.as_array().into_iter().flatten() {
        let href = connection["link"].as_str().unwrap_or_default();
        if href.is_empty()
            && connection["tooltip"]
                .as_str()
                .unwrap_or_default()
                .is_empty()
        {
            continue;
        }
        let route = connection["route"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| Point {
                x: number(p, "x"),
                y: number(p, "y"),
            })
            .collect::<Vec<_>>();
        let size = Size {
            width: number(connection, "labelWidth"),
            height: number(connection, "labelHeight"),
        };
        let position =
            LabelPosition::from_d2_name(connection["labelPosition"].as_str().unwrap_or_default());
        let origin = d2_rs_core::route_label::top_left(&route, position, number(connection, "strokeWidth"), number(connection, "labelPercentage"), size)
            .ok_or_else(|| ExportError::Pdf(format!("scene: connection {:?} field \"labelPosition\" has invalid value {:?}: does not resolve to a finite metadata hit region", connection["id"].as_str().unwrap_or_default(), connection["labelPosition"].as_str().unwrap_or_default())))?;
        regions.push(DocumentLinkMetadata {
            tooltip: connection["tooltip"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            board_target: is_board_target(href),
            region: DocumentLinkRegion {
                href: href.to_owned(),
                rect: Rect {
                    origin: Point {
                        x: origin.x.round(),
                        y: origin.y.round(),
                    },
                    size,
                },
            },
        });
    }
    Ok(regions)
}

fn is_board_target(href: &str) -> bool {
    d2_rs_language::parse_key(href.as_bytes())
        .ok()
        .and_then(|key| key.string_ida())
        .is_some_and(|parts| parts.first().is_some_and(|part| part == "root"))
}

/// lib/pdf.GetFillRGB resolves page fill against the catalog, independently of
/// the transparent scene frame. Per-board root fills survive the raster clear.
pub fn page_fill(graph: &Graph, options: &RenderOptions) -> Result<[u8; 3], ExportError> {
    let target = d2_rs_render::target_root_metadata(graph, options)
        .map_err(|e| ExportError::Pdf(e.to_string()))?;
    let fill = target["fill"].as_str().unwrap_or_default();
    if fill.is_empty() || fill.eq_ignore_ascii_case("transparent") {
        return Ok([255; 3]);
    }
    let palette = theme::palette(options.theme_id.unwrap_or(0))
        .or_else(|| theme::palette(0))
        .unwrap();
    let resolved = theme::resolve_color(palette, fill).unwrap_or(fill);
    let color = resolved
        .parse::<svgtypes::Color>()
        .map_err(|e| ExportError::Pdf(e.to_string()))?;
    Ok([color.red, color.green, color.blue])
}
