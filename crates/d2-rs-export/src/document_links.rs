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
    go_get_fill_rgb(options.theme_id.unwrap_or(0), fill)
}

/// lib/pdf/pdf.go `GetFillRGB`: theme codes resolve against the catalog,
/// `color.Name2RGB` names short-circuit unless they map to the zero RGB
/// (black), and everything else must pass `color.Hex2RGB`. Gradients, CSS
/// functions and short hex forms therefore keep Go's results and errors.
fn go_get_fill_rgb(theme_id: i64, fill: &str) -> Result<[u8; 3], ExportError> {
    let lowered = String::from_utf8(d2_rs_core::ir::go_lowercase(fill.as_bytes()))
        .unwrap_or_else(|_| fill.to_ascii_lowercase());
    if fill.is_empty() || lowered == "transparent" {
        return Ok([255; 3]);
    }
    let resolved = if d2_rs_target::theme::is_theme_color(fill) {
        // d2themescatalog.Find returns the zero theme for unknown IDs, whose
        // empty palette value then fails Hex2RGB.
        theme::palette(theme_id)
            .and_then(|palette| theme::resolve_color(palette, fill))
            .unwrap_or_default()
    } else {
        // lib/color/color.go namedRgbMap spells this entry "muintcream".
        let rgb = match lowered.as_str() {
            "mintcream" => None,
            "muintcream" => Some([245, 255, 250]),
            name => d2_rs_math::svg_color::css_named_rgb(name),
        };
        if let Some(rgb) = rgb.filter(|rgb| *rgb != [0; 3]) {
            return Ok(rgb);
        }
        fill
    };
    go_hex2rgb(resolved)
}

/// lib/color/color.go `Hex2RGB`.
fn go_hex2rgb(hex: &str) -> Result<[u8; 3], ExportError> {
    let digits = match hex.as_bytes() {
        [b'#', _, _, _, ..] => &hex[1..],
        _ => return Err(ExportError::Pdf(format!("cannot parse hex color {hex}"))),
    };
    // strconv.ParseUint(hex, 16, 32): bare hex digits only, at most 32 bits.
    let quoted = serde_json::to_string(digits).unwrap_or_default();
    if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ExportError::Pdf(format!(
            "strconv.ParseUint: parsing {quoted}: invalid syntax"
        )));
    }
    let values = u64::from_str_radix(digits, 16)
        .ok()
        .filter(|value| *value <= u64::from(u32::MAX))
        .ok_or_else(|| {
            ExportError::Pdf(format!(
                "strconv.ParseUint: parsing {quoted}: value out of range"
            ))
        })?;
    Ok([(values >> 16) as u8, (values >> 8) as u8, values as u8])
}

#[cfg(test)]
mod go_fill_rgb_tests {
    use super::*;

    #[test]
    fn get_fill_rgb_follows_lib_pdf_and_lib_color() {
        assert_eq!(go_get_fill_rgb(0, ""), Ok([255; 3]));
        assert_eq!(go_get_fill_rgb(0, "TransParent"), Ok([255; 3]));
        assert_eq!(go_get_fill_rgb(0, "Red"), Ok([255, 0, 0]));
        assert_eq!(go_get_fill_rgb(0, "#F7F8FE"), Ok([0xf7, 0xf8, 0xfe]));
        // Hex2RGB parses the digits as one integer: "#fff" is 0x000fff.
        assert_eq!(go_get_fill_rgb(0, "#fff"), Ok([0x00, 0x0f, 0xff]));
        // Name2RGB's zero result (black) falls through to Hex2RGB.
        assert!(go_get_fill_rgb(0, "black").is_err());
        assert!(go_get_fill_rgb(0, "mintcream").is_err());
        assert!(go_get_fill_rgb(0, "#123456789").is_err());
        assert_eq!(
            go_get_fill_rgb(0, "linear-gradient(#000000, #ffffff)"),
            Err(ExportError::Pdf(
                "cannot parse hex color linear-gradient(#000000, #ffffff)".into()
            ))
        );
        let n7 = theme::resolve_color(theme::palette(0).unwrap(), "N7").unwrap();
        assert_eq!(go_get_fill_rgb(0, "N7"), go_hex2rgb(n7));
    }
}
