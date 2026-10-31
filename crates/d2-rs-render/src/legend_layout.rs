// SPDX-License-Identifier: MPL-2.0
//! Source d2scenebuild/legend.go measurement and integer placement.
use super::*;
use d2_rs_text::measure::{self, TrueTypeFont};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct ItemDimensions {
    pub width: f64,
    pub height: f64,
}
#[derive(Debug)]
pub(super) struct LegendLayout {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub max_label_width: f64,
    pub shapes: Vec<ItemDimensions>,
    pub connections: Vec<ItemDimensions>,
}

fn font(family: Option<&str>) -> &'static TrueTypeFont {
    static SANS: OnceLock<TrueTypeFont> = OnceLock::new();
    static MONO: OnceLock<TrueTypeFont> = OnceLock::new();
    match family {
        Some("mono") => MONO.get_or_init(|| {
            measure::bundled_source_code_pro_regular().expect("bundled legend mono font")
        }),
        _ => SANS.get_or_init(|| {
            measure::bundled_source_sans_pro_regular().expect("bundled legend sans font")
        }),
    }
}
fn dimensions(text: &str, family: Option<&str>) -> ItemDimensions {
    let metrics = font(family).measure_with_fallback(text, LEGEND_FONT_SIZE, font(Some("mono")));
    ItemDimensions {
        width: metrics.width.ceil(),
        height: metrics.height.ceil(),
    }
}

pub(super) fn measure(
    legend: &Legend,
    bounds: (f64, f64, f64, f64),
    mono_theme: bool,
) -> Option<LegendLayout> {
    if legend.shapes.is_empty() && legend.connections.is_empty() {
        return None;
    }
    let shapes = legend
        .shapes
        .iter()
        .map(|s| {
            if s.label.is_empty() {
                ItemDimensions::default()
            } else {
                dimensions(
                    &s.label,
                    if mono_theme {
                        Some("mono")
                    } else {
                        s.font.as_deref()
                    },
                )
            }
        })
        .collect::<Vec<_>>();
    let connections = legend
        .connections
        .iter()
        .map(|s| {
            if s.label.is_empty() {
                ItemDimensions::default()
            } else {
                dimensions(&s.label, s.font.as_deref().or(mono_theme.then_some("mono")))
            }
        })
        .collect::<Vec<_>>();
    let mut height = LEGEND_PADDING + LEGEND_FONT_SIZE + LEGEND_ITEM_SPACING;
    let mut max_label_width: f64 = 0.0;
    let mut count = 0;
    for (label, d) in legend.shapes.iter().map(|s| &s.label).zip(&shapes).chain(
        legend
            .connections
            .iter()
            .map(|s| &s.label)
            .zip(&connections),
    ) {
        if label.is_empty() {
            continue;
        }
        height += d.height.max(LEGEND_ICON_SIZE) + LEGEND_ITEM_SPACING;
        max_label_width = max_label_width.max(d.width);
        count += 1;
    }
    if count > 0 {
        height -= (LEGEND_ITEM_SPACING / 2.0).trunc();
    }
    height += if count > 0 && !legend.connections.is_empty() {
        LEGEND_PADDING * 3.0 / 2.0
    } else {
        LEGEND_PADDING * 6.0 / 5.0
    };
    let width = LEGEND_PADDING * 3.0 + LEGEND_ICON_SIZE + max_label_width;
    Some(LegendLayout {
        x: bounds.2.trunc() + LEGEND_CORNER_PADDING,
        y: (bounds.3.trunc() - height).max(bounds.1.trunc()),
        width,
        height,
        max_label_width,
        shapes,
        connections,
    })
}
impl LegendLayout {
    pub fn expand_viewbox(&self, mut box_: (f64, f64, f64, f64), pad: f64) -> (f64, f64, f64, f64) {
        if self.max_label_width <= 0.0 {
            return box_;
        }
        if box_.0 + box_.2 < self.x + self.width {
            box_.2 = self.x + self.width - box_.0 + (pad / 2.0).trunc();
        }
        if self.y < box_.1 {
            box_.3 += box_.1 - self.y;
            box_.1 = self.y;
        }
        if box_.1 + box_.3 < self.y + self.height {
            box_.3 = self.y + self.height - box_.1 + (pad / 2.0).trunc();
        }
        box_
    }
}

pub(super) fn rendered_text(value: &str) -> String {
    // SVG collapses only these ASCII whitespace runes; Unicode spacing stays.
    let mut result = String::new();
    let mut pending = false;
    for ch in value.chars() {
        if matches!(ch, ' ' | '\t' | '\n' | '\r') {
            if !result.is_empty() {
                pending = true;
            }
        } else {
            if pending {
                result.push(' ');
                pending = false;
            }
            result.push(ch);
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_text(
    svg: &mut String,
    value: &str,
    x: f64,
    y: f64,
    size: u32,
    bold: bool,
    mono: bool,
    id: &str,
    precision: bool,
) {
    let text = rendered_text(value);
    let class = match (mono, bold) {
        (true, true) => "text-mono-bold",
        (true, false) => "text-mono",
        (false, true) => "text-bold",
        _ => "text",
    };
    let retained = if precision {
        format!(
            " id=\"d2-rs-native-legend-{id}\" xml:space=\"preserve\" data-d2-rs-text-raw=\"true\" data-d2-rs-text-x-bits=\"{:016x}\" data-d2-rs-text-y-bits=\"{:016x}\"",
            x.to_bits(),
            y.to_bits()
        )
    } else {
        String::new()
    };
    let weight = if bold { " font-weight=\"bold\"" } else { "" };
    let _ = writeln!(
        svg,
        "<text class=\"{class}\"{retained} x=\"{}\" y=\"{}\" fill=\"#000000\" font-size=\"{size}\"{weight}>{}</text>",
        number(x),
        number(y),
        escape_xml_text(&text)
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_text_with_target(
    svg: &mut String,
    value: &str,
    x: f64,
    y: f64,
    size: u32,
    bold: bool,
    mono: bool,
    id: &str,
    precision: bool,
    target: bool,
) {
    if !target {
        return draw_text(svg, value, x, y, size, bold, mono, id, precision);
    }
    // Go selects a dedicated bold family. Our shared font family registers
    // that same concrete face at weight 700, so request it explicitly.
    let class = if bold { "text-bold" } else { "text" };
    let weight = if bold { " font-weight=\"bold\"" } else { "" };
    let _ = write!(
        svg,
        "<text class=\"{class}\" x=\"{}\" y=\"{}\"{weight} style=\"font-size: {size}px;\">{}</text>",
        number(x),
        number(y),
        escape_xml_text(value)
    );
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod source_contract {
    use super::*;
    #[test]
    fn complete_original_go_legend_layouts_and_label_origins() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/legend-layout-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 63);
        for row in rows.as_array().unwrap() {
            let source = &row["legend"];
            let strings = |key: &str| source[key].as_array().map(Vec::as_slice).unwrap_or(&[]);
            let legend = Legend {
                source_graph: None,
                label: source["label"].as_str().unwrap_or_default().into(),
                shapes: strings("shapes")
                    .iter()
                    .map(|s| d2_rs_core::LegendShape {
                        source_node: None,
                        font: s["fontFamily"].as_str().map(str::to_owned),
                        label: s["label"].as_str().unwrap_or_default().into(),
                        shape: ShapeKind::Rectangle,
                        fill: None,
                        stroke: None,
                    })
                    .collect(),
                connections: strings("connections")
                    .iter()
                    .map(|s| d2_rs_core::LegendConnection {
                        font: s["fontFamily"].as_str().map(str::to_owned),
                        label: s["label"].as_str().unwrap_or_default().into(),
                        source_arrow: false,
                        target_arrow: false,
                        stroke: None,
                        stroke_width: None,
                        stroke_dash: None,
                        opacity: None,
                        animated: false,
                        ..Default::default()
                    })
                    .collect(),
            };
            let b = row["bounds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect::<Vec<_>>();
            let bounds = (b[0], b[1], b[2], b[3]);
            let layout = measure(&legend, bounds, false).unwrap();
            let nodes = row["nodes"].as_array().unwrap();
            let panel =
                &nodes.iter().find(|n| n["id"] == "legend:panel").unwrap()["primitive"]["Box"];
            assert_eq!(
                (layout.x, layout.y, layout.width, layout.height),
                (
                    panel["X"].as_f64().unwrap(),
                    panel["Y"].as_f64().unwrap(),
                    panel["Width"].as_f64().unwrap(),
                    panel["Height"].as_f64().unwrap()
                ),
                "{}",
                row["name"]
            );
            let pad = row["pad"].as_f64().unwrap();
            let actual = layout.expand_viewbox(
                (
                    b[0] - pad,
                    b[1] - pad,
                    b[2] - b[0] + 2.0 * pad,
                    b[3] - b[1] + 2.0 * pad,
                ),
                pad,
            );
            let v = &row["viewBox"];
            assert_eq!(
                actual,
                (
                    v["X"].as_f64().unwrap(),
                    v["Y"].as_f64().unwrap(),
                    v["Width"].as_f64().unwrap(),
                    v["Height"].as_f64().unwrap()
                ),
                "{}",
                row["name"]
            );
            let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\">");
            let palette = active_palette(None);
            draw_legend(
                &mut svg,
                &legend,
                bounds,
                0.0,
                0.0,
                PaletteView::new(palette, &BTreeMap::new()),
                false,
                false,
                &mut std::collections::BTreeSet::new(),
                None,
            );
            svg.push_str("</svg>");
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let actual = doc
                .descendants()
                .filter(|n| n.has_tag_name("text"))
                .collect::<Vec<_>>();
            let expected = nodes
                .iter()
                .filter(|n| n.get("origin").is_some())
                .collect::<Vec<_>>();
            assert_eq!(actual.len(), expected.len());
            for (a, e) in actual.into_iter().zip(expected) {
                assert_eq!(
                    (
                        a.attribute("x").unwrap().parse::<f64>().unwrap(),
                        a.attribute("y").unwrap().parse::<f64>().unwrap()
                    ),
                    (
                        e["origin"]["X"].as_f64().unwrap(),
                        e["origin"]["Y"].as_f64().unwrap()
                    ),
                    "{} {}",
                    row["name"],
                    e["id"]
                );
                assert_eq!(
                    a.text().unwrap_or_default(),
                    e["text"].as_str().unwrap(),
                    "{} {}",
                    row["name"],
                    e["id"]
                );
            }
        }
    }
}
