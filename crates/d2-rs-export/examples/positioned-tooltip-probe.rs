// SPDX-License-Identifier: MPL-2.0
//! Capture actual native PNGs for data-driven direct-target comparisons.
use d2_rs_core::{Graph, LayoutReport, LayoutResult, Node, Point, Rect, Score, Size};
use d2_rs_language::compiler::SourceNodeStyle;
use std::{collections::BTreeMap, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 4);
    let rows: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let directory = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&directory)?;
    let mut outcomes = Vec::new();
    for row in rows.as_array().unwrap() {
        let name = row["name"].as_str().unwrap();
        let shape = &row["shape"];
        let size = Size {
            width: shape["width"].as_f64().unwrap(),
            height: shape["height"].as_f64().unwrap(),
        };
        let mut graph = Graph::default();
        let id = graph.add_node(Node::new("probe", size));
        let rect = Rect {
            origin: Point {
                x: shape["pos"]["x"].as_f64().unwrap(),
                y: shape["pos"]["y"].as_f64().unwrap(),
            },
            size,
        };
        let field = |key: &str| {
            shape
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        let styles = BTreeMap::from([(
            id,
            SourceNodeStyle {
                fill: field("fill"),
                stroke: field("stroke"),
                stroke_width: Some(shape["strokeWidth"].to_string()),
                tooltip: field("tooltip"),
                tooltip_position: field("tooltipPosition"),
                link: field("link"),
                pretty_link: Some(
                    shape
                        .get("prettyLink")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                ),
                font: field("fontFamily"),
                ..Default::default()
            },
        )]);
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            boxes: BTreeMap::from([(id, rect)]),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            edge_label_percentage_commits: Default::default(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 0,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let mut outcome = serde_json::json!({"name":name,"error":""});
        match d2_rs_export::render_png(&graph, &result, &BTreeMap::new(), &styles) {
            Ok(png) => {
                let file = format!("{name}.png");
                std::fs::write(directory.join(&file), png)?;
                outcome["file"] = file.into();
            }
            Err(error) => {
                outcome["error"] = error.to_string().into();
            }
        };
        outcomes.push(outcome);
    }
    std::fs::write(&args[3], serde_json::to_vec_pretty(&outcomes)?)?;
    Ok(())
}
