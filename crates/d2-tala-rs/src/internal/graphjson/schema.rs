// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Declared graphjson fields from pinned D2 v0.9.0 representation.go.
use super::decode::Kind;
pub(super) fn fields(name: &str) -> &'static [(&'static str, Kind)] {
    match name {
        "SerializedLabel" => &[
            ("Text", Kind::String),
            ("Position", Kind::String),
            ("Width", Kind::Float),
            ("Height", Kind::Float),
        ],
        "SerializedIcon" => &[("Position", Kind::String)],
        "SerializedPoint" => &[("x", Kind::Float), ("y", Kind::Float)],
        "SerializedScalar" => &[("value", Kind::String)],
        "SerializedStyle" => &[
            ("opacity", Kind::Pointer(&Kind::Struct("SerializedScalar"))),
            ("stroke", Kind::Pointer(&Kind::Struct("SerializedScalar"))),
            ("fill", Kind::Pointer(&Kind::Struct("SerializedScalar"))),
            (
                "fillPattern",
                Kind::Pointer(&Kind::Struct("SerializedScalar")),
            ),
            (
                "strokeWidth",
                Kind::Pointer(&Kind::Struct("SerializedScalar")),
            ),
            (
                "strokeDash",
                Kind::Pointer(&Kind::Struct("SerializedScalar")),
            ),
            (
                "borderRadius",
                Kind::Pointer(&Kind::Struct("SerializedScalar")),
            ),
            ("shadow", Kind::Pointer(&Kind::Struct("SerializedScalar"))),
            ("3d", Kind::Pointer(&Kind::Struct("SerializedScalar"))),
            ("multiple", Kind::Pointer(&Kind::Struct("SerializedScalar"))),
            ("font", Kind::Pointer(&Kind::Struct("SerializedScalar"))),
            ("fontSize", Kind::Pointer(&Kind::Struct("SerializedScalar"))),
            (
                "fontColor",
                Kind::Pointer(&Kind::Struct("SerializedScalar")),
            ),
            ("animated", Kind::Pointer(&Kind::Struct("SerializedScalar"))),
            ("bold", Kind::Pointer(&Kind::Struct("SerializedScalar"))),
            ("italic", Kind::Pointer(&Kind::Struct("SerializedScalar"))),
            (
                "underline",
                Kind::Pointer(&Kind::Struct("SerializedScalar")),
            ),
            ("filled", Kind::Pointer(&Kind::Struct("SerializedScalar"))),
            (
                "doubleBorder",
                Kind::Pointer(&Kind::Struct("SerializedScalar")),
            ),
            (
                "textTransform",
                Kind::Pointer(&Kind::Struct("SerializedScalar")),
            ),
        ],
        "SerializedNode" => &[
            ("id", Kind::Int),
            ("d2id", Kind::Pointer(&Kind::String)),
            ("isInvisible", Kind::Bool),
            ("width", Kind::Float),
            ("height", Kind::Float),
            ("topLeft", Kind::Pointer(&Kind::Struct("SerializedPoint"))),
            (
                "fixedTopLeft",
                Kind::Pointer(&Kind::Struct("SerializedPoint")),
            ),
            ("desiredWidth", Kind::Pointer(&Kind::Float)),
            ("desiredHeight", Kind::Pointer(&Kind::Float)),
            ("nears", Kind::List(&Kind::Int)),
            ("fontSize", Kind::Pointer(&Kind::NativeInt)),
            ("shapeType", Kind::String),
            ("label", Kind::Pointer(&Kind::Struct("SerializedLabel"))),
            ("icon", Kind::Pointer(&Kind::Struct("SerializedIcon"))),
            ("numColumns", Kind::NativeInt),
            ("is3D", Kind::Bool),
            ("isMultiple", Kind::Bool),
            ("forceHierarchy", Kind::Bool),
        ],
        "SerializedEdge" => &[
            ("id", Kind::Int),
            ("originalId", Kind::Pointer(&Kind::Int)),
            ("d2id", Kind::Pointer(&Kind::String)),
            ("from", Kind::Int),
            ("isInvisible", Kind::Bool),
            ("to", Kind::Int),
            (
                "points",
                Kind::List(&Kind::Pointer(&Kind::Struct("SerializedPoint"))),
            ),
            ("minWidth", Kind::NativeInt),
            ("minHeight", Kind::NativeInt),
            ("labelPercentage", Kind::Float),
            ("label", Kind::Pointer(&Kind::Struct("SerializedLabel"))),
            ("fromTableColumnIndex", Kind::Pointer(&Kind::NativeInt)),
            ("toTableColumnIndex", Kind::Pointer(&Kind::NativeInt)),
            ("style", Kind::Struct("SerializedStyle")),
            ("sourceArrowhead", Kind::String),
            ("targetArrowhead", Kind::String),
            (
                "sourceArrowheadLabel",
                Kind::Pointer(&Kind::Struct("SerializedLabel")),
            ),
            (
                "targetArrowheadLabel",
                Kind::Pointer(&Kind::Struct("SerializedLabel")),
            ),
        ],
        "SerializedEdgeAbduction" => &[
            ("Edge", Kind::Int),
            ("OriginallyFromNode", Kind::Int),
            ("OriginallyToNode", Kind::Int),
            ("CurrentFromNode", Kind::Int),
            ("CurrentToNode", Kind::Int),
        ],
        "SerializedCluster" => &[
            ("Nodes", Kind::List(&Kind::Int)),
            ("Arrangement", Kind::String),
            ("DesiredArrangement", Kind::String),
            (
                "EdgeAbductions",
                Kind::List(&Kind::Struct("SerializedEdgeAbduction")),
            ),
            ("Container", Kind::Int),
            ("FixedSize", Kind::Bool),
            ("Padding", Kind::Float),
        ],
        "SerializedSequence" => &[
            ("Nodes", Kind::List(&Kind::Int)),
            (
                "EdgeAbductions",
                Kind::List(&Kind::Struct("SerializedEdgeAbduction")),
            ),
            ("Container", Kind::Int),
        ],
        "SerializedTree" => &[
            ("Node", Kind::Struct("SerializedNode")),
            ("Children", Kind::List(&Kind::Struct("SerializedTree"))),
            ("SentinelEdge", Kind::Struct("SerializedEdge")),
            ("Orientation", Kind::NativeInt),
        ],
        "SerializedHierarchy" => &[("Levels", Kind::Map(&Kind::NativeInt))],
        "SerializedGraph" => &[
            ("nodes", Kind::List(&Kind::Struct("SerializedNode"))),
            ("edges", Kind::List(&Kind::Struct("SerializedEdge"))),
            ("CellSize", Kind::Float),
            ("isRootHierarchy", Kind::Bool),
            ("containers", Kind::Map(&Kind::List(&Kind::Int))),
            ("hubs", Kind::Map(&Kind::List(&Kind::Int))),
            ("clusters", Kind::Map(&Kind::Struct("SerializedCluster"))),
            ("sequences", Kind::Map(&Kind::Struct("SerializedSequence"))),
            ("clusterVessels", Kind::Map(&Kind::Struct("SerializedNode"))),
            (
                "sequenceVessels",
                Kind::Map(&Kind::Struct("SerializedNode")),
            ),
            (
                "trees",
                Kind::Map(&Kind::List(&Kind::Struct("SerializedTree"))),
            ),
            (
                "hierarchies",
                Kind::List(&Kind::Struct("SerializedHierarchy")),
            ),
            ("directions", Kind::Map(&Kind::String)),
            ("crossingCost", Kind::Float),
            ("turnCost", Kind::Float),
            ("nonCenterPortCost", Kind::Float),
        ],
        _ => unreachable!("undeclared wire struct"),
    }
}
