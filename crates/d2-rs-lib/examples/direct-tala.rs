// SPDX-License-Identifier: MPL-2.0
//! Development-only adapter for D2 v0.9.0's internal TALA graph fixtures.
//!
//! The public plugin protocol carries `objects` and `edges`. The upstream
//! engine regression corpus instead serializes `layoutgraph.Graph` as
//! `nodes`/`edges`/`containers`. Keeping this adapter separate makes it
//! possible to compare the two engines without widening the shipped protocol.

use d2_rs_core::{
    Direction, Edge, EdgeArrowheads, EdgeArrows, EdgeLabel, EdgeStyle, Graph, LabelPosition, Node,
    Point, ShapeKind, Size,
};
use d2_rs_tala::{Engine, LayoutOptions};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};

fn object<'a>(value: &'a Value, name: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .get(name)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("direct graph member {name:?} must be an object"))
}

fn array<'a>(value: &'a Value, name: &str) -> Result<&'a Vec<Value>, String> {
    value
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("direct graph member {name:?} must be an array"))
}

fn integer(value: &Value, name: &str) -> Result<i64, String> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .ok_or_else(|| format!("direct graph field {name:?} must be an integer"))
}

fn number(value: &Value, name: &str) -> Result<f64, String> {
    value
        .as_f64()
        .ok_or_else(|| format!("direct graph field {name:?} must be a number"))
}

fn point(value: &Value, name: &str) -> Result<Point, String> {
    let map = value
        .as_object()
        .ok_or_else(|| format!("direct graph field {name:?} must be a point"))?;
    Ok(Point {
        x: number(
            map.get("x").ok_or_else(|| format!("missing {name}.x"))?,
            name,
        )?,
        y: number(
            map.get("y").ok_or_else(|| format!("missing {name}.y"))?,
            name,
        )?,
    })
}

fn optional_point(value: Option<&Value>, name: &str) -> Result<Option<Point>, String> {
    match value {
        Some(Value::Null) | None => Ok(None),
        Some(value) => point(value, name).map(Some),
    }
}

fn size(map: &Map<String, Value>) -> Result<Size, String> {
    Ok(Size {
        width: number(
            map.get("width")
                .ok_or_else(|| "missing node.width".to_owned())?,
            "width",
        )?,
        height: number(
            map.get("height")
                .ok_or_else(|| "missing node.height".to_owned())?,
            "height",
        )?,
    })
}

fn shape(value: Option<&Value>) -> ShapeKind {
    match value.and_then(Value::as_str).unwrap_or_default() {
        "Square" => ShapeKind::Square,
        "RealSquare" => ShapeKind::RealSquare,
        "Parallelogram" => ShapeKind::Parallelogram,
        "Document" => ShapeKind::Document,
        "Cylinder" => ShapeKind::Cylinder,
        "Queue" => ShapeKind::Queue,
        "Page" => ShapeKind::Page,
        "Package" => ShapeKind::Package,
        "Step" => ShapeKind::Step,
        "Callout" => ShapeKind::Callout,
        "StoredData" => ShapeKind::StoredData,
        "Person" => ShapeKind::Person,
        "C4Person" => ShapeKind::C4Person,
        "Diamond" => ShapeKind::Diamond,
        "Oval" => ShapeKind::Oval,
        "Circle" => ShapeKind::Circle,
        "Hexagon" => ShapeKind::Hexagon,
        "Cloud" => ShapeKind::Cloud,
        "Table" => ShapeKind::SqlTable,
        "Class" => ShapeKind::Class,
        "Text" => ShapeKind::Text,
        "Code" => ShapeKind::Code,
        "Image" => ShapeKind::Image,
        _ => ShapeKind::Rectangle,
    }
}

fn direction(value: Option<&Value>) -> Option<Direction> {
    match value.and_then(Value::as_str)?.to_ascii_lowercase().as_str() {
        "right" => Some(Direction::Right),
        "left" => Some(Direction::Left),
        "up" => Some(Direction::Up),
        "down" => Some(Direction::Down),
        _ => None,
    }
}

fn label(value: Option<&Value>) -> Option<EdgeLabel> {
    let map = value?.as_object()?;
    let text = map
        .get("Text")
        .or_else(|| map.get("text"))
        .and_then(Value::as_str)?
        .to_owned();
    let width = map
        .get("Width")
        .or_else(|| map.get("width"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let height = map
        .get("Height")
        .or_else(|| map.get("height"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let position = map
        .get("Position")
        .or_else(|| map.get("position"))
        .and_then(Value::as_str)
        .map(LabelPosition::from_d2_name)
        .unwrap_or_default();
    Some(EdgeLabel {
        text,
        size: Size { width, height },
        position,
        percentage: 0.5,
    })
}

fn style(value: Option<&Value>) -> EdgeStyle {
    let Some(map) = value.and_then(Value::as_object) else {
        return EdgeStyle::default();
    };
    let value = |name: &str| {
        map.get(name)
            .and_then(Value::as_object)
            .and_then(|value| value.get("Value"))
            .or_else(|| {
                map.get(name)
                    .and_then(Value::as_object)
                    .and_then(|value| value.get("value"))
            })
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    EdgeStyle {
        language: None,
        opacity: value("opacity"),
        stroke: value("stroke"),
        stroke_width: value("strokeWidth"),
        stroke_dash: value("strokeDash"),
        border_radius: value("borderRadius"),
        animated: value("animated"),
        font_color: value("fontColor"),
        font: value("font"),
        italic: value("italic"),
        underline: value("underline"),
        text_transform: value("textTransform"),
        fill: value("fill"),
        filled: value("filled"),
    }
}

fn decode(input: &Value) -> Result<Graph, String> {
    let nodes = array(input, "nodes")?;
    let mut graph = Graph::default();
    let mut ids = HashMap::<i64, d2_rs_core::NodeId>::new();
    let mut node_ids = Vec::with_capacity(nodes.len());

    for (index, value) in nodes.iter().enumerate() {
        let map = value
            .as_object()
            .ok_or_else(|| format!("node at index {index} must be an object"))?;
        let id = integer(
            map.get("id")
                .ok_or_else(|| "node is missing id".to_owned())?,
            "id",
        )?;
        if ids.contains_key(&id) {
            return Err(format!("duplicate direct node ID {id}"));
        }
        let node_size = size(map)?;
        let mut node = Node::new(id.to_string(), node_size);
        node.declared_size = Some(node_size);
        node.shape = shape(map.get("shapeType"));
        node.label_size = map.get("label").and_then(|value| {
            let map = value.as_object()?;
            Some(Size {
                width: map.get("Width")?.as_f64()?,
                height: map.get("Height")?.as_f64()?,
            })
        });
        node.label_position = map
            .get("label")
            .and_then(|value| value.as_object())
            .and_then(|map| map.get("Position"))
            .and_then(Value::as_str)
            .map(LabelPosition::from_d2_name)
            .unwrap_or_default();
        node.font_size = map
            .get("fontSize")
            .and_then(Value::as_i64)
            .and_then(|value| u32::try_from(value).ok());
        node.locked_position = optional_point(map.get("fixedTopLeft"), "fixedTopLeft")?;
        let initial_position = optional_point(map.get("topLeft"), "topLeft")?;
        node.fixed_width = map
            .get("desiredWidth")
            .is_some_and(|value| !value.is_null());
        node.fixed_height = map
            .get("desiredHeight")
            .is_some_and(|value| !value.is_null());
        node.force_hierarchy = map
            .get("forceHierarchy")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        node.is_3d = map.get("is3D").and_then(Value::as_bool).unwrap_or(false);
        node.is_multiple = map
            .get("isMultiple")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(icon) = map.get("icon").and_then(Value::as_object) {
            node.has_icon = true;
            node.icon_position = icon
                .get("Position")
                .and_then(Value::as_str)
                .map(LabelPosition::from_d2_name);
        }
        let dense = graph.add_node(node);
        graph.tala_id_overrides[dense.0 as usize] = Some(id);
        graph.set_node_invisible(
            dense,
            map.get("isInvisible")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        );
        graph.set_initial_position(dense, initial_position);
        if let Some(columns) = map.get("numColumns").and_then(Value::as_i64) {
            graph.set_table_column_count(dense, usize::try_from(columns).ok());
        }
        ids.insert(id, dense);
        node_ids.push(id);
    }

    let containers = object(input, "containers")?;
    let mut child_order = BTreeMap::new();
    for (parent_key, children) in containers {
        let parent = if parent_key == "0" {
            None
        } else {
            Some(
                *ids.get(
                    &parent_key
                        .parse::<i64>()
                        .map_err(|_| format!("container key {parent_key:?} is not an integer"))?,
                )
                .ok_or_else(|| format!("container references unknown node {parent_key:?}"))?,
            )
        };
        let values = children
            .as_array()
            .ok_or_else(|| format!("container {parent_key:?} must contain an array"))?;
        let mut order = Vec::with_capacity(values.len());
        for child in values {
            let child_id = integer(child, "container child")?;
            let child_dense = *ids
                .get(&child_id)
                .ok_or_else(|| format!("container references unknown child {child_id}"))?;
            graph.node_mut(child_dense).expect("known node").parent = parent;
            order.push(child_dense);
        }
        child_order.insert(parent, order);
    }
    graph.set_hierarchy_children_order(child_order);
    graph.root_hierarchy = input
        .get("isRootHierarchy")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Some(directions) = input.get("directions").and_then(Value::as_object) {
        for (node_key, value) in directions {
            let parsed = direction(Some(value));
            if node_key == "0" {
                if let Some(value) = parsed {
                    graph.direction = value;
                    graph.explicit_direction = Some(value);
                }
            } else if let Ok(id) = node_key.parse::<i64>()
                && let Some(node) = ids.get(&id).and_then(|id| graph.node_mut(*id))
            {
                node.direction = parsed;
            }
        }
    }

    let edges = array(input, "edges")?;
    for (index, value) in edges.iter().enumerate() {
        let map = value
            .as_object()
            .ok_or_else(|| format!("edge at index {index} must be an object"))?;
        let id = integer(
            map.get("id")
                .ok_or_else(|| "edge is missing id".to_owned())?,
            "id",
        )?;
        let from = integer(
            map.get("from")
                .ok_or_else(|| "edge is missing from".to_owned())?,
            "from",
        )?;
        let to = integer(
            map.get("to")
                .ok_or_else(|| "edge is missing to".to_owned())?,
            "to",
        )?;
        let edge_id = graph.add_edge(Edge {
            source: *ids
                .get(&from)
                .ok_or_else(|| format!("edge references unknown node {from}"))?,
            target: *ids
                .get(&to)
                .ok_or_else(|| format!("edge references unknown node {to}"))?,
        });
        graph.tala_edge_ids[edge_id.0 as usize] = Some(id);
        graph.set_edge_min_dimensions(
            edge_id,
            map.get("minWidth").and_then(Value::as_f64),
            map.get("minHeight").and_then(Value::as_f64),
        );
        graph.set_edge_route(
            edge_id,
            map.get("points")
                .and_then(Value::as_array)
                .map(|points| {
                    points
                        .iter()
                        .filter_map(|value| point(value, "edge point").ok())
                        .collect()
                })
                .unwrap_or_default(),
        );
        graph.set_edge_arrows(
            edge_id,
            EdgeArrows {
                source: map
                    .get("sourceArrowhead")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.is_empty()),
                target: map
                    .get("targetArrowhead")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.is_empty()),
            },
        );
        graph.set_edge_arrowheads(
            edge_id,
            EdgeArrowheads {
                source: map
                    .get("sourceArrowhead")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
                target: map
                    .get("targetArrowhead")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
            },
        );
        if let Some(label) = label(map.get("label")) {
            graph.set_edge_label(edge_id, Some(label));
        }
        graph.set_edge_style(edge_id, style(map.get("style")));
        graph.tala_edge_invisible[edge_id.0 as usize] =
            map.get("isInvisible").and_then(Value::as_bool);
    }

    for (index, value) in nodes.iter().enumerate() {
        let map = value.as_object().expect("validated node");
        if let Some(nears) = map.get("nears").and_then(Value::as_array) {
            let mut decoded = nears
                .iter()
                .filter_map(|value| integer(value, "near").ok())
                .filter_map(|id| ids.get(&id).copied());
            graph.nodes[index].near = decoded.next();
            graph.near_targets[index].extend(decoded);
        }
    }
    Ok(graph)
}

#[derive(Serialize)]
struct OutputNode {
    id: i64,
    width: f64,
    height: f64,
    top_left: Option<Point>,
}

#[derive(Serialize)]
struct OutputEdge {
    id: i64,
    from: i64,
    to: i64,
    points: Vec<Point>,
}

fn run() -> Result<Vec<u8>, String> {
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .map_err(|error| error.to_string())?;
    let document: Value = serde_json::from_slice(&input).map_err(|error| error.to_string())?;
    let graph = decode(&document)?;
    let seed = std::env::args()
        .skip(1)
        .collect::<Vec<_>>()
        .first()
        .map(String::as_str)
        .unwrap_or("1")
        .parse::<i64>()
        .map_err(|error| format!("invalid seed: {error}"))?;
    let result = Engine
        .layout(
            &graph,
            &LayoutOptions {
                seeds: vec![seed],
                max_concurrency: 0,
            },
        )
        .map_err(|error| error.to_string())?;
    let nodes = graph
        .nodes()
        .map(|(dense, node)| {
            let id = graph.tala_id_overrides[dense.0 as usize].unwrap_or(dense.0 as i64);
            let rect = result.boxes.get(&dense).copied();
            OutputNode {
                id,
                width: rect.map_or(node.size.width, |rect| rect.size.width),
                height: rect.map_or(node.size.height, |rect| rect.size.height),
                top_left: rect.map(|rect| rect.origin),
            }
        })
        .collect::<Vec<_>>();
    let edges = graph
        .edges()
        .filter(|(dense, _)| result.routes.contains_key(dense))
        .map(|(dense, edge)| OutputEdge {
            id: graph.tala_edge_ids[dense.0 as usize].unwrap_or(dense.0 as i64),
            from: graph.tala_id_overrides[edge.source.0 as usize].unwrap_or(edge.source.0 as i64),
            to: graph.tala_id_overrides[edge.target.0 as usize].unwrap_or(edge.target.0 as i64),
            points: result.routes.get(&dense).cloned().unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&serde_json::json!({"nodes": nodes, "edges": edges}))
        .map_err(|error| error.to_string())
}

fn main() {
    match run() {
        Ok(output) => {
            if let Err(error) = std::io::stdout().write_all(&output) {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
