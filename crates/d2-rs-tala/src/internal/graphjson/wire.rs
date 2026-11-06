// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Wire representation from D2 v0.9.0 graphjson/representation.go.
//! Does not restore graph ownership or implement Go's full decoding semantics.
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}
fn is_zero<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}
fn empty_vec<T>(value: &Option<Vec<T>>) -> bool {
    value.as_ref().is_none_or(Vec::is_empty)
}
fn empty_map<K, V>(value: &Option<BTreeMap<K, V>>) -> bool {
    value.as_ref().is_none_or(BTreeMap::is_empty)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SerializedLabel {
    #[serde(rename = "Text", deserialize_with = "null_default")]
    pub text: String,
    #[serde(rename = "Position", deserialize_with = "null_default")]
    pub position: String,
    #[serde(rename = "Width", deserialize_with = "null_default")]
    pub width: f64,
    #[serde(rename = "Height", deserialize_with = "null_default")]
    pub height: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SerializedIcon {
    #[serde(rename = "Position", deserialize_with = "null_default")]
    pub position: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SerializedPoint {
    #[serde(rename = "x", deserialize_with = "null_default")]
    pub x: f64,
    #[serde(rename = "y", deserialize_with = "null_default")]
    pub y: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SerializedScalar {
    #[serde(rename = "value", deserialize_with = "null_default")]
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SerializedStyle {
    #[serde(rename = "opacity", skip_serializing_if = "Option::is_none")]
    pub opacity: Option<SerializedScalar>,
    #[serde(rename = "stroke", skip_serializing_if = "Option::is_none")]
    pub stroke: Option<SerializedScalar>,
    #[serde(rename = "fill", skip_serializing_if = "Option::is_none")]
    pub fill: Option<SerializedScalar>,
    #[serde(rename = "fillPattern", skip_serializing_if = "Option::is_none")]
    pub fill_pattern: Option<SerializedScalar>,
    #[serde(rename = "strokeWidth", skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<SerializedScalar>,
    #[serde(rename = "strokeDash", skip_serializing_if = "Option::is_none")]
    pub stroke_dash: Option<SerializedScalar>,
    #[serde(rename = "borderRadius", skip_serializing_if = "Option::is_none")]
    pub border_radius: Option<SerializedScalar>,
    #[serde(rename = "shadow", skip_serializing_if = "Option::is_none")]
    pub shadow: Option<SerializedScalar>,
    #[serde(rename = "3d", skip_serializing_if = "Option::is_none")]
    pub three_dee: Option<SerializedScalar>,
    #[serde(rename = "multiple", skip_serializing_if = "Option::is_none")]
    pub multiple: Option<SerializedScalar>,
    #[serde(rename = "font", skip_serializing_if = "Option::is_none")]
    pub font: Option<SerializedScalar>,
    #[serde(rename = "fontSize", skip_serializing_if = "Option::is_none")]
    pub font_size: Option<SerializedScalar>,
    #[serde(rename = "fontColor", skip_serializing_if = "Option::is_none")]
    pub font_color: Option<SerializedScalar>,
    #[serde(rename = "animated", skip_serializing_if = "Option::is_none")]
    pub animated: Option<SerializedScalar>,
    #[serde(rename = "bold", skip_serializing_if = "Option::is_none")]
    pub bold: Option<SerializedScalar>,
    #[serde(rename = "italic", skip_serializing_if = "Option::is_none")]
    pub italic: Option<SerializedScalar>,
    #[serde(rename = "underline", skip_serializing_if = "Option::is_none")]
    pub underline: Option<SerializedScalar>,
    #[serde(rename = "filled", skip_serializing_if = "Option::is_none")]
    pub filled: Option<SerializedScalar>,
    #[serde(rename = "doubleBorder", skip_serializing_if = "Option::is_none")]
    pub double_border: Option<SerializedScalar>,
    #[serde(rename = "textTransform", skip_serializing_if = "Option::is_none")]
    pub text_transform: Option<SerializedScalar>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SerializedNode {
    #[serde(rename = "id", deserialize_with = "null_default")]
    pub id: i64,
    #[serde(rename = "d2id", skip_serializing_if = "Option::is_none")]
    pub d2_id: Option<String>,
    #[serde(
        rename = "isInvisible",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub is_invisible: bool,
    #[serde(rename = "width", deserialize_with = "null_default")]
    pub width: f64,
    #[serde(rename = "height", deserialize_with = "null_default")]
    pub height: f64,
    #[serde(rename = "topLeft")]
    pub top_left: Option<SerializedPoint>,
    #[serde(rename = "fixedTopLeft", skip_serializing_if = "Option::is_none")]
    pub fixed_top_left: Option<SerializedPoint>,
    #[serde(rename = "desiredWidth", skip_serializing_if = "Option::is_none")]
    pub desired_width: Option<f64>,
    #[serde(rename = "desiredHeight", skip_serializing_if = "Option::is_none")]
    pub desired_height: Option<f64>,
    #[serde(rename = "nears", skip_serializing_if = "empty_vec")]
    pub nears: Option<Vec<i64>>,
    #[serde(rename = "fontSize", skip_serializing_if = "Option::is_none")]
    pub font_size: Option<i64>,
    #[serde(rename = "shapeType", deserialize_with = "null_default")]
    pub shape_type: String,
    #[serde(rename = "label", skip_serializing_if = "Option::is_none")]
    pub label: Option<SerializedLabel>,
    #[serde(rename = "icon", skip_serializing_if = "Option::is_none")]
    pub icon: Option<SerializedIcon>,
    #[serde(
        rename = "numColumns",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub num_columns: i64,
    #[serde(
        rename = "is3D",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub is3_d: bool,
    #[serde(
        rename = "isMultiple",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub is_multiple: bool,
    #[serde(
        rename = "forceHierarchy",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub force_hierarchy: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SerializedEdge {
    #[serde(rename = "id", deserialize_with = "null_default")]
    pub id: i64,
    #[serde(rename = "originalId", skip_serializing_if = "Option::is_none")]
    pub original_id: Option<i64>,
    #[serde(rename = "d2id", skip_serializing_if = "Option::is_none")]
    pub d2_id: Option<String>,
    #[serde(rename = "from", deserialize_with = "null_default")]
    pub from_node: i64,
    #[serde(
        rename = "isInvisible",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub is_invisible: bool,
    #[serde(rename = "to", deserialize_with = "null_default")]
    pub to_node: i64,
    #[serde(rename = "points")]
    pub points: Option<Vec<Option<SerializedPoint>>>,
    #[serde(rename = "minWidth", deserialize_with = "null_default")]
    pub min_width: i64,
    #[serde(rename = "minHeight", deserialize_with = "null_default")]
    pub min_height: i64,
    #[serde(rename = "labelPercentage", deserialize_with = "null_default")]
    pub label_percentage: f64,
    #[serde(rename = "label", skip_serializing_if = "Option::is_none")]
    pub label: Option<SerializedLabel>,
    #[serde(
        rename = "fromTableColumnIndex",
        skip_serializing_if = "Option::is_none"
    )]
    pub from_table_column_index: Option<i64>,
    #[serde(rename = "toTableColumnIndex", skip_serializing_if = "Option::is_none")]
    pub to_table_column_index: Option<i64>,
    #[serde(rename = "style", deserialize_with = "null_default")]
    pub style: SerializedStyle,
    #[serde(rename = "sourceArrowhead", deserialize_with = "null_default")]
    pub source_arrowhead: String,
    #[serde(rename = "targetArrowhead", deserialize_with = "null_default")]
    pub target_arrowhead: String,
    #[serde(
        rename = "sourceArrowheadLabel",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_arrowhead_label: Option<SerializedLabel>,
    #[serde(
        rename = "targetArrowheadLabel",
        skip_serializing_if = "Option::is_none"
    )]
    pub target_arrowhead_label: Option<SerializedLabel>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SerializedEdgeAbduction {
    #[serde(rename = "Edge", deserialize_with = "null_default")]
    pub edge: i64,
    #[serde(rename = "OriginallyFromNode", deserialize_with = "null_default")]
    pub originally_from_node: i64,
    #[serde(rename = "OriginallyToNode", deserialize_with = "null_default")]
    pub originally_to_node: i64,
    #[serde(rename = "CurrentFromNode", deserialize_with = "null_default")]
    pub current_from_node: i64,
    #[serde(rename = "CurrentToNode", deserialize_with = "null_default")]
    pub current_to_node: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SerializedCluster {
    #[serde(rename = "Nodes")]
    pub nodes: Option<Vec<i64>>,
    #[serde(rename = "Arrangement", deserialize_with = "null_default")]
    pub arrangement: String,
    #[serde(rename = "DesiredArrangement", deserialize_with = "null_default")]
    pub desired_arrangement: String,
    #[serde(rename = "EdgeAbductions")]
    pub edge_abductions: Option<Vec<SerializedEdgeAbduction>>,
    #[serde(rename = "Container", deserialize_with = "null_default")]
    pub container: i64,
    #[serde(rename = "FixedSize", deserialize_with = "null_default")]
    pub fixed_size: bool,
    #[serde(rename = "Padding", deserialize_with = "null_default")]
    pub padding: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SerializedSequence {
    #[serde(rename = "Nodes")]
    pub nodes: Option<Vec<i64>>,
    #[serde(rename = "EdgeAbductions")]
    pub edge_abductions: Option<Vec<SerializedEdgeAbduction>>,
    #[serde(rename = "Container", deserialize_with = "null_default")]
    pub container: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SerializedTree {
    #[serde(rename = "Node", deserialize_with = "null_default")]
    pub node: SerializedNode,
    #[serde(rename = "Children")]
    pub children: Option<Vec<SerializedTree>>,
    #[serde(rename = "SentinelEdge", deserialize_with = "null_default")]
    pub sentinel_edge: SerializedEdge,
    #[serde(rename = "Orientation", deserialize_with = "null_default")]
    pub orientation: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SerializedHierarchy {
    #[serde(rename = "Levels")]
    pub levels: Option<BTreeMap<i64, i64>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SerializedGraph {
    #[serde(rename = "nodes")]
    pub nodes: Option<Vec<SerializedNode>>,
    #[serde(rename = "edges")]
    pub edges: Option<Vec<SerializedEdge>>,
    #[serde(rename = "CellSize", deserialize_with = "null_default")]
    pub cell_size: f64,
    #[serde(
        rename = "isRootHierarchy",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub is_root_hierarchy: bool,
    #[serde(rename = "containers")]
    pub containers: Option<BTreeMap<i64, Option<Vec<i64>>>>,
    #[serde(rename = "hubs", skip_serializing_if = "empty_map")]
    pub hubs: Option<BTreeMap<i64, Option<Vec<i64>>>>,
    #[serde(rename = "clusters", skip_serializing_if = "empty_map")]
    pub clusters: Option<BTreeMap<i64, SerializedCluster>>,
    #[serde(rename = "sequences", skip_serializing_if = "empty_map")]
    pub sequences: Option<BTreeMap<i64, SerializedSequence>>,
    #[serde(rename = "clusterVessels", skip_serializing_if = "empty_map")]
    pub cluster_vessels: Option<BTreeMap<i64, SerializedNode>>,
    #[serde(rename = "sequenceVessels", skip_serializing_if = "empty_map")]
    pub sequence_vessels: Option<BTreeMap<i64, SerializedNode>>,
    #[serde(rename = "trees", skip_serializing_if = "empty_map")]
    pub trees: Option<BTreeMap<i64, Option<Vec<SerializedTree>>>>,
    #[serde(rename = "hierarchies", skip_serializing_if = "empty_vec")]
    pub hierarchies: Option<Vec<SerializedHierarchy>>,
    #[serde(rename = "directions", skip_serializing_if = "empty_map")]
    pub directions: Option<BTreeMap<i64, String>>,
    #[serde(rename = "crossingCost", deserialize_with = "null_default")]
    pub crossing_cost: f64,
    #[serde(rename = "turnCost", deserialize_with = "null_default")]
    pub turn_cost: f64,
    #[serde(rename = "nonCenterPortCost", deserialize_with = "null_default")]
    pub non_center_port_cost: f64,
}
