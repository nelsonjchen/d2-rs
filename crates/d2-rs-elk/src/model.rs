// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Source-shaped ELK graph records shared by native and WASM adapters.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A node in the public ELK graph model.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ElkNode {
    /// Stable ELK node identifier.
    pub id: String,
    /// Relative horizontal position.
    #[serde(default)]
    pub x: f64,
    /// Relative vertical position.
    #[serde(default)]
    pub y: f64,
    /// Node width.
    pub width: f64,
    /// Node height.
    pub height: f64,
    /// Nested child nodes.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub children: Vec<ElkNode>,
    /// Edges stored on this node in the public JSON tree.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub edges: Vec<ElkEdge>,
    /// Fixed or dynamically distributed ports.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub ports: Vec<ElkPort>,
    /// Node labels.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub labels: Vec<ElkLabel>,
    /// ELK layout options, when the node has a configured profile.
    #[serde(rename = "layoutOptions", skip_serializing_if = "Option::is_none")]
    pub layout_options: Option<Value>,
}

/// A port in the public ELK graph model.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ElkPort {
    /// Stable port identifier.
    pub id: String,
    /// Relative horizontal position.
    #[serde(default)]
    pub x: f64,
    /// Relative vertical position.
    #[serde(default)]
    pub y: f64,
    /// Port width.
    pub width: f64,
    /// Port height.
    pub height: f64,
    /// Port-specific ELK options.
    #[serde(rename = "layoutOptions", default)]
    pub layout_options: Value,
}

/// An ELK identifier preserving string versus signed integer representation.
///
/// Pinned graphjson.ID accepts integral JSON lexemes within signed64 bounds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ElkId {
    /// A string identifier, including an explicitly empty string.
    String(String),
    /// A signed64 integer identifier.
    Integer(i64),
}

impl<'de> Deserialize<'de> for ElkId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // RawValue keeps lexical integer validity: -0 is accepted, whereas
        // 1.0 and 1e3 are rejected even though their f64 values are integral.
        let raw = Box::<serde_json::value::RawValue>::deserialize(deserializer)?;
        let token = raw.get().trim();
        if token.starts_with('"') {
            return serde_json::from_str::<String>(token)
                .map(Self::String)
                .map_err(serde::de::Error::custom);
        }
        token.parse::<i64>().map(Self::Integer).map_err(|_| {
            serde::de::Error::custom(format!(
                "graph element id must be a string or integer: {token}"
            ))
        })
    }
}

/// A node or edge label in the public ELK graph model.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ElkLabel {
    /// Original optional label identifier, retained through layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<ElkId>,
    /// Label text.
    pub text: String,
    /// Relative horizontal position.
    #[serde(default)]
    pub x: f64,
    /// Relative vertical position.
    #[serde(default)]
    pub y: f64,
    /// Label width.
    pub width: f64,
    /// Label height.
    pub height: f64,
    /// Optional label-specific ELK options.
    #[serde(rename = "layoutOptions", skip_serializing_if = "Option::is_none")]
    pub layout_options: Option<Value>,
}

/// A routed edge section returned by ELK.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ElkEdgeSection {
    /// Stable section identifier assigned by ELK's transfer phase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Section start point.
    #[serde(rename = "startPoint")]
    pub start: ElkPoint,
    /// Section end point.
    #[serde(rename = "endPoint")]
    pub end: ElkPoint,
    /// Interior bend points.
    #[serde(rename = "bendPoints", skip_serializing_if = "Vec::is_empty", default)]
    pub bends: Vec<ElkPoint>,
    /// Shape or port ID at the incoming endpoint.
    #[serde(rename = "incomingShape", skip_serializing_if = "Option::is_none")]
    pub incoming_shape: Option<String>,
    /// Shape or port ID at the outgoing endpoint.
    #[serde(rename = "outgoingShape", skip_serializing_if = "Option::is_none")]
    pub outgoing_shape: Option<String>,
}

/// A point in an ELK edge section.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct ElkPoint {
    /// Horizontal coordinate.
    pub x: f64,
    /// Vertical coordinate.
    pub y: f64,
}

// Go's []Point accepts null slices, and Point accepts null objects and
// missing/null coordinates as zero. Arrays and scalar points remain invalid.
fn deserialize_junction_points<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<ElkPoint>, D::Error> {
    Ok(Option::<Vec<ElkPoint>>::deserialize(deserializer)?.unwrap_or_default())
}

impl<'de> Deserialize<'de> for ElkPoint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PointVisitor;
        impl<'de> serde::de::Visitor<'de> for PointVisitor {
            type Value = ElkPoint;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an ELK point object or null")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<ElkPoint, E> {
                Ok(ElkPoint::default())
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<ElkPoint, M::Error> {
                let mut point = ElkPoint::default();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        key if key.eq_ignore_ascii_case("x") => {
                            point.x = map.next_value::<Option<f64>>()?.unwrap_or(0.0)
                        }
                        key if key.eq_ignore_ascii_case("y") => {
                            point.y = map.next_value::<Option<f64>>()?.unwrap_or(0.0)
                        }
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(point)
            }
        }
        deserializer.deserialize_any(PointVisitor)
    }
}

/// An edge in the public ELK graph model.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ElkEdge {
    /// Routed junctions in the owning edge container's coordinate frame.
    #[serde(
        rename = "junctionPoints",
        default,
        deserialize_with = "deserialize_junction_points",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub junction_points: Vec<ElkPoint>,
    /// Stable edge identifier.
    pub id: String,
    /// Source node or port IDs.
    pub sources: Vec<String>,
    /// Target node or port IDs.
    pub targets: Vec<String>,
    /// Routed sections, when layout has been run.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub sections: Vec<ElkEdgeSection>,
    /// Edge labels.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub labels: Vec<ElkLabel>,
    /// Owning container identifier.
    #[serde(default)]
    pub container: String,
    /// Optional ELK edge properties, including layered priority weights.
    #[serde(
        rename = "layoutOptions",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub layout_options: Option<Value>,
}

/// The root of the public ELK graph model.
#[derive(Clone, Debug)]
pub struct ElkGraph {
    /// Root horizontal position; omitted input stays absent until transfer.
    pub x: Option<f64>,
    /// Root vertical position; omitted input stays absent until transfer.
    pub y: Option<f64>,
    /// Root layout width, absent until computed.
    pub width: Option<f64>,
    /// Root layout height, absent until computed.
    pub height: Option<f64>,
    /// Root graph identifier.
    pub id: String,
    /// Root ELK layout options; null represents a missing/nil map.
    pub layout_options: Value,
    /// Root children.
    pub children: Vec<ElkNode>,
    /// Root edges.
    pub edges: Vec<ElkEdge>,
    /// Whether JSON supplied a non-null child list, including an empty list.
    pub children_present: bool,
    /// Whether JSON supplied a non-null edge list, including an empty list.
    pub edges_present: bool,
}

impl<'de> Deserialize<'de> for ElkGraph {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Input {
            #[serde(default)]
            x: Option<f64>,
            #[serde(default)]
            y: Option<f64>,
            #[serde(default)]
            width: Option<f64>,
            #[serde(default)]
            height: Option<f64>,
            id: String,
            #[serde(rename = "layoutOptions", default)]
            layout_options: Value,
            #[serde(default)]
            children: Option<Vec<ElkNode>>,
            #[serde(default)]
            edges: Option<Vec<ElkEdge>>,
        }
        let input = Input::deserialize(deserializer)?;
        Ok(Self {
            x: input.x,
            y: input.y,
            width: input.width,
            height: input.height,
            id: input.id,
            layout_options: input.layout_options,
            children_present: input.children.is_some(),
            edges_present: input.edges.is_some(),
            children: input.children.unwrap_or_default(),
            edges: input.edges.unwrap_or_default(),
        })
    }
}

impl Serialize for ElkGraph {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        if let Some(value) = self.x {
            map.serialize_entry("x", &value)?;
        }
        if let Some(value) = self.y {
            map.serialize_entry("y", &value)?;
        }
        if let Some(value) = self.width {
            map.serialize_entry("width", &value)?;
        }
        if let Some(value) = self.height {
            map.serialize_entry("height", &value)?;
        }
        map.serialize_entry("id", &self.id)?;
        if !self.layout_options.is_null() {
            map.serialize_entry("layoutOptions", &self.layout_options)?;
        }
        if self.children_present || !self.children.is_empty() {
            map.serialize_entry("children", &self.children)?;
        }
        if self.edges_present || !self.edges.is_empty() {
            map.serialize_entry("edges", &self.edges)?;
        }
        map.end()
    }
}

impl ElkGraph {
    /// Codec.importEdges visits the root's edges before each child subtree.
    pub(crate) fn edges_in_source_order(&self) -> Vec<&ElkEdge> {
        fn collect<'a>(nodes: &'a [ElkNode], edges: &mut Vec<&'a ElkEdge>) {
            for node in nodes {
                edges.extend(&node.edges);
                collect(&node.children, edges);
            }
        }
        let mut edges = self.edges.iter().collect::<Vec<_>>();
        collect(&self.children, &mut edges);
        edges
    }
}

/// Stable bindings between public edge lists and the algorithm worklist.
/// Like Codec.edgeJSON/edgeParents, this keeps JSON ownership separate from
/// an edge's computed layout scope. Drop restores every list even on errors
/// or unwinding; the pipeline only borrows an edge slice and cannot resize it.
pub(crate) struct EdgeBindings<'a> {
    pub(crate) graph: &'a mut ElkGraph,
    pub(crate) edges: Vec<ElkEdge>,
    owners: Vec<(Vec<usize>, usize)>,
    root_count: usize,
}

impl<'a> EdgeBindings<'a> {
    pub(crate) fn take(graph: &'a mut ElkGraph) -> Self {
        fn collect(
            nodes: &mut [ElkNode],
            path: &mut Vec<usize>,
            edges: &mut Vec<ElkEdge>,
            owners: &mut Vec<(Vec<usize>, usize)>,
        ) {
            for (index, node) in nodes.iter_mut().enumerate() {
                path.push(index);
                if !node.edges.is_empty() {
                    owners.push((path.clone(), node.edges.len()));
                    edges.append(&mut node.edges);
                }
                collect(&mut node.children, path, edges, owners);
                path.pop();
            }
        }
        let mut edges = std::mem::take(&mut graph.edges);
        let root_count = edges.len();
        let mut owners = Vec::new();
        collect(
            &mut graph.children,
            &mut Vec::new(),
            &mut edges,
            &mut owners,
        );
        Self {
            graph,
            edges,
            owners,
            root_count,
        }
    }
}

impl Drop for EdgeBindings<'_> {
    fn drop(&mut self) {
        for (path, count) in self.owners.iter().rev() {
            let edges = self.edges.split_off(self.edges.len() - count);
            let mut nodes = &mut self.graph.children;
            for (depth, index) in path.iter().copied().enumerate() {
                if depth + 1 == path.len() {
                    nodes[index].edges = edges;
                    break;
                }
                nodes = &mut nodes[index].children;
            }
        }
        debug_assert_eq!(self.edges.len(), self.root_count);
        self.graph.edges = std::mem::take(&mut self.edges);
    }
}

#[cfg(test)]
mod tests {
    use super::{ElkEdge, ElkGraph, ElkNode};
    use serde_json::json;

    #[test]
    fn graph_model_uses_elk_json_keys_and_omits_empty_sections() {
        let graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: String::new(),
            layout_options: json!({"elk.algorithm": "layered"}),
            children: Vec::new(),
            edges: vec![ElkEdge {
                junction_points: Vec::new(),
                id: "e".into(),
                sources: vec!["a".into()],
                targets: vec!["b".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: String::new(),
                layout_options: None,
            }],
        };
        let value = serde_json::to_value(graph).unwrap();
        assert_eq!(value["layoutOptions"]["elk.algorithm"], "layered");
        assert!(value["edges"][0].get("sections").is_none());
        assert!(value["edges"][0].get("labels").is_none());
    }

    #[test]
    fn graph_model_accepts_pre_layout_nodes_and_labels_without_positions() {
        let graph: ElkGraph = serde_json::from_value(json!({
            "id": "root",
            "layoutOptions": {"elk.algorithm": "layered"},
            "children": [{
                "id": "node",
                "width": 40,
                "height": 30,
                "labels": [{"text": "label", "width": 20, "height": 10}]
            }]
        }))
        .unwrap();
        let node: &ElkNode = &graph.children[0];
        assert_eq!((node.x, node.y), (0.0, 0.0));
        assert_eq!((node.labels[0].x, node.labels[0].y), (0.0, 0.0));
        let edge: ElkEdge = serde_json::from_value(json!({
            "id": "edge",
            "sources": ["node"],
            "targets": ["node"]
        }))
        .unwrap();
        assert!(edge.container.is_empty());
    }
    #[test]
    fn original_go_point_null_defaults_case_fold_and_last_field_win() {
        for (raw, x, y) in [
            ("null", 0.0, 0.0),
            ("{}", 0.0, 0.0),
            (r#"{"X":1,"x":2,"Y":null}"#, 2.0, 0.0),
            (r#"{"x":-0,"y":3,"extra":true}"#, -0.0, 3.0),
        ] {
            let point: super::ElkPoint = serde_json::from_str(raw).unwrap();
            assert_eq!(point.x.to_bits(), f64::to_bits(x));
            assert_eq!(point.y.to_bits(), f64::to_bits(y));
        }
        for raw in ["[]", "true", r#"{"X":"bad"}"#] {
            assert!(serde_json::from_str::<super::ElkPoint>(raw).is_err());
        }
    }
}
