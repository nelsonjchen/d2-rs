// SPDX-License-Identifier: MPL-2.0
//! Shared types for the d2-rs workspace: the D2 AST and IR models, measured
//! graphs, geometry, layout results, themes and the Go-compatible float,
//! sorting and string-quoting helpers the other crates use to match upstream Go
//! D2.
//!
//! The `oracle-fixtures` feature compiles tests that read regenerated Go-oracle
//! artifacts.
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU8, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use thiserror::Error;

pub mod ast;
mod go_float;
#[doc(hidden)]
pub use go_float::fused_mul_add;
#[doc(hidden)]
pub use go_float::go_fmadd;
#[doc(hidden)]
pub use go_float::go_round;
pub mod grid;
#[cfg(windows)]
#[doc(hidden)]
pub mod windows_io;
pub use go_float::{go_json_float, parse_d2_float, parse_d2_float_result};
mod go_quote;
/// Quote diagnostic bytes using pinned Go string/Unicode semantics.
#[doc(hidden)]
pub use go_quote::quote as quote_go_bytes;
#[doc(hidden)]
pub mod go_sort;
#[doc(hidden)]
pub mod identity;
pub mod ir;
pub mod layout_guard;
pub mod route_label;
pub mod shape_geometry;
mod source_style;
pub mod stable_sort;
pub mod theme;
pub use source_style::{SourceNodeStyle, SourceSqlColumn, SourceSqlText, SourceTargetAccents};

pub const NODE_GAP: f64 = 20.0;

/// Cooperative cancellation handle shared by layout calls and their workers.
/// Explicit cancellation and deadline expiration retain distinct Go context
/// errors. The first cause wins; children inherit their parent's cancellation
/// without allowing a child's deadline to cancel its parent.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    state: Arc<CancellationState>,
}

#[derive(Default)]
struct CancellationState {
    // 0 is active, 1 is explicit cancellation, 2 is an expired deadline.
    reason: AtomicU8,
    children: Mutex<CancellationChildren>,
    propagated: Condvar,
    // Descendants keep every intermediate context alive, as Go Context does.
    parent: Option<Arc<CancellationState>>,
}

impl std::fmt::Debug for CancellationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellationState")
            .field("reason", &self.reason)
            .field("has_parent", &self.parent.is_some())
            .finish_non_exhaustive()
    }
}

impl Drop for CancellationState {
    fn drop(&mut self) {
        // Strong references point only toward ancestors. Release a uniquely
        // owned chain iteratively, including when its contexts are still active.
        let mut parent = self.parent.take();
        while let Some(ancestor) = parent {
            let Some(mut ancestor) = Arc::into_inner(ancestor) else {
                break;
            };
            parent = ancestor.parent.take();
        }
    }
}

#[derive(Debug, Default)]
struct CancellationChildren {
    nodes: Vec<Weak<CancellationState>>,
    complete: bool,
}

impl CancellationToken {
    /// Creates an active token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an independently cancellable child which inherits this token's cause.
    #[must_use]
    pub fn child(&self) -> Self {
        let child = Self {
            state: Arc::new(CancellationState {
                reason: AtomicU8::new(0),
                children: Mutex::default(),
                propagated: Condvar::new(),
                parent: Some(Arc::clone(&self.state)),
            }),
        };
        let mut children = self
            .state
            .children
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let reason = self.state.reason.load(AtomicOrdering::Acquire);
        if reason == 0 {
            children.nodes.retain(|child| child.strong_count() != 0);
            children.nodes.push(Arc::downgrade(&child.state));
        }
        drop(children);
        if reason != 0 {
            child.finish(reason);
        }
        child
    }

    fn finish(&self, reason: u8) {
        enum Pending {
            Cancel(Arc<CancellationState>),
            Complete(Arc<CancellationState>),
        }
        let mut pending = vec![Pending::Cancel(Arc::clone(&self.state))];
        while let Some(operation) = pending.pop() {
            match operation {
                Pending::Cancel(state) => {
                    let mut children = state.children.lock().unwrap_or_else(|e| e.into_inner());
                    if state.reason.load(AtomicOrdering::Acquire) != 0 {
                        // Go's cancelCtx holds its mutex through child propagation.
                        // Cause publication alone does not complete a cancel call.
                        while !children.complete {
                            children = state
                                .propagated
                                .wait(children)
                                .unwrap_or_else(|e| e.into_inner());
                        }
                        continue;
                    }
                    state.reason.store(reason, AtomicOrdering::Release);
                    // Complete each parent only after its descendants. Keep this
                    // iterative so long context chains do not exhaust Rust stacks.
                    pending.push(Pending::Complete(Arc::clone(&state)));
                    pending.extend(
                        children
                            .nodes
                            .drain(..)
                            .filter_map(|child| child.upgrade())
                            .map(Pending::Cancel),
                    );
                }
                Pending::Complete(state) => {
                    let mut children = state.children.lock().unwrap_or_else(|e| e.into_inner());
                    children.complete = true;
                    state.propagated.notify_all();
                }
            }
        }
    }

    /// Requests explicit cancellation. An earlier deadline remains the cause.
    /// Returns after propagation to registered descendants has completed.
    pub fn cancel(&self) {
        self.finish(1);
    }

    /// Reports deadline expiration. An earlier cancellation remains the cause.
    /// Returns after propagation to registered descendants has completed.
    /// A host timer calls this when its deadline expires; it does not start a timer.
    pub fn expire(&self) {
        self.finish(2);
    }

    /// Returns whether the token's Done state has been reached.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.reason.load(AtomicOrdering::Acquire) != 0
    }

    /// Returns the original context cause, once cancellation is observed.
    #[must_use]
    pub fn error(&self) -> Option<LayoutError> {
        match self.state.reason.load(AtomicOrdering::Acquire) {
            1 => Some(LayoutError::Cancelled),
            2 => Some(LayoutError::DeadlineExceeded),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
/// Stable index of a node within a [`Graph`].
pub struct NodeId(
    /// Zero-based insertion index.
    pub u32,
);

impl NodeId {
    /// The synthetic source graph root is an endpoint, not an ordinary object.
    /// It never appears in `Graph::nodes` or consumes an arena index.
    pub const ROOT: Self = Self(u32::MAX);
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
/// Stable index of an edge within a [`Graph`].
pub struct EdgeId(
    /// Zero-based insertion index.
    pub u32,
);

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
/// A two-dimensional coordinate in layout pixels.
pub struct Point {
    /// Horizontal coordinate.
    pub x: f64,
    /// Vertical coordinate.
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
/// Width and height in layout pixels.
pub struct Size {
    /// Horizontal extent.
    pub width: f64,
    /// Vertical extent.
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
/// Insets measured inward from the four sides of a rectangle.
pub struct Insets {
    /// Top inset.
    pub top: f64,
    /// Right inset.
    pub right: f64,
    /// Bottom inset.
    pub bottom: f64,
    /// Left inset.
    pub left: f64,
}

impl Insets {
    /// Creates equal insets on all four sides.
    pub const fn uniform(value: f64) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Side of a node occupied by an external decoration.
pub enum ExternalSide {
    /// Above the node.
    Top,
    /// To the right of the node.
    Right,
    /// Below the node.
    Bottom,
    /// To the left of the node.
    Left,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
/// Alignment of an external decoration along its selected side.
pub enum ExternalAlignment {
    /// Align with the side's leading corner.
    Start,
    /// Center along the side.
    #[default]
    Center,
    /// Align with the side's trailing corner.
    End,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
/// Rule used to align children inside a container.
pub enum ContentAlignment {
    /// Respect the container's content insets.
    #[default]
    Padding,
    /// Center the content rectangle.
    Center,
    /// Apply cloud-shape content alignment.
    Cloud,
    /// Apply diamond-shape content alignment.
    Diamond,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Rendered node shape used by sizing, port selection, and border tracing.
pub enum ShapeKind {
    /// Axis-aligned rectangle with independent width and height.
    #[default]
    Rectangle,
    /// Rectangular D2 square shape. Its width and height remain independent;
    /// the internal `RealSquare` shape is the unit-aspect variant.
    Square,
    /// Internal unit-aspect square used by TALA's `RealSquare` shape.
    RealSquare,
    /// Slanted quadrilateral used by flowchart input/output nodes.
    Parallelogram,
    /// Document outline with a curved lower border.
    Document,
    /// Cylindrical database or storage outline.
    Cylinder,
    /// Queue/storage outline with curved vertical sides.
    Queue,
    /// Page outline with a folded corner.
    Page,
    /// Package outline with a tab above the main body.
    Package,
    /// Flowchart step with offset side edges.
    Step,
    /// Speech or annotation box with a callout pointer.
    Callout,
    /// Stored-data flowchart outline.
    StoredData,
    /// Person silhouette using D2's ordinary person geometry.
    Person,
    /// Person silhouette using the C4 model rendering.
    #[serde(rename = "c4-person", alias = "c4_person")]
    C4Person,
    /// Rhombus commonly used for decisions.
    Diamond,
    /// Ellipse with independent width and height.
    Oval,
    /// Ellipse whose adapter-provided dimensions normally have equal axes.
    Circle,
    /// Six-sided polygon.
    Hexagon,
    /// Cloud outline with shape-specific content fitting.
    Cloud,
    /// SQL table whose rows and endpoint columns influence routing.
    SqlTable,
    /// Class or UML-style record with internal compartments.
    Class,
    /// Text-only node; it cannot act as a routing container.
    Text,
    /// Code block; it cannot act as a routing container.
    Code,
    /// Image node; it cannot act as a routing container.
    Image,
}

impl ShapeKind {
    /// Whether this shape requires equal width and height during layout.
    pub fn has_unit_aspect_ratio(self) -> bool {
        matches!(self, Self::RealSquare | Self::Circle)
    }

    /// Returns whether standalone routing may infer children inside this shape.
    pub const fn can_contain(self) -> bool {
        !matches!(
            self,
            Self::Image | Self::Code | Self::SqlTable | Self::Class | Self::Text
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
/// Label or icon rendered outside a node rectangle.
pub struct ExternalLabel {
    /// Rendered dimensions.
    pub size: Size,
    /// Side on which the decoration appears.
    pub side: ExternalSide,
    #[serde(default)]
    /// Alignment along [`Self::side`].
    pub alignment: ExternalAlignment,
    /// Whether the adapter selected the placement automatically.
    pub automatic: bool,
    /// Whether layout must reserve clearance for the decoration.
    pub reserve_space: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
/// Axis-aligned rectangle in layout pixels.
pub struct Rect {
    /// Top-left coordinate.
    pub origin: Point,
    /// Rectangle dimensions.
    pub size: Size,
}

impl ExternalLabel {
    /// Returns this automatic decoration positioned at a layout-selected
    /// outside label position. Explicitly authored positions keep their
    /// declared side and alignment.
    pub fn at_position(mut self, position: LabelPosition) -> Self {
        if !self.automatic {
            return self;
        }
        let placement = match position {
            LabelPosition::OutsideTopLeft => (ExternalSide::Top, ExternalAlignment::Start),
            LabelPosition::OutsideTopCenter => (ExternalSide::Top, ExternalAlignment::Center),
            LabelPosition::OutsideTopRight => (ExternalSide::Top, ExternalAlignment::End),
            LabelPosition::OutsideRightTop => (ExternalSide::Right, ExternalAlignment::Start),
            LabelPosition::OutsideRightMiddle => (ExternalSide::Right, ExternalAlignment::Center),
            LabelPosition::OutsideRightBottom => (ExternalSide::Right, ExternalAlignment::End),
            LabelPosition::OutsideBottomLeft => (ExternalSide::Bottom, ExternalAlignment::Start),
            LabelPosition::OutsideBottomCenter => (ExternalSide::Bottom, ExternalAlignment::Center),
            LabelPosition::OutsideBottomRight => (ExternalSide::Bottom, ExternalAlignment::End),
            LabelPosition::OutsideLeftTop => (ExternalSide::Left, ExternalAlignment::Start),
            LabelPosition::OutsideLeftMiddle => (ExternalSide::Left, ExternalAlignment::Center),
            LabelPosition::OutsideLeftBottom => (ExternalSide::Left, ExternalAlignment::End),
            _ => return self,
        };
        self.side = placement.0;
        self.alignment = placement.1;
        self
    }

    pub fn top_left(self, rect: Rect, padding: f64) -> Point {
        match (self.side, self.alignment) {
            (ExternalSide::Top, ExternalAlignment::Start) => Point {
                x: rect.origin.x - padding,
                y: rect.origin.y - padding - self.size.height,
            },
            (ExternalSide::Top, ExternalAlignment::Center) => Point {
                x: rect.origin.x + (rect.size.width - self.size.width) / 2.0,
                y: rect.origin.y - padding - self.size.height,
            },
            (ExternalSide::Top, ExternalAlignment::End) => Point {
                x: rect.right() - self.size.width - padding,
                y: rect.origin.y - padding - self.size.height,
            },
            (ExternalSide::Right, ExternalAlignment::Start) => Point {
                x: rect.right() + padding,
                y: rect.origin.y + padding,
            },
            (ExternalSide::Right, ExternalAlignment::Center) => Point {
                x: rect.right() + padding,
                y: rect.origin.y + (rect.size.height - self.size.height) / 2.0,
            },
            (ExternalSide::Right, ExternalAlignment::End) => Point {
                x: rect.right() + padding,
                y: rect.bottom() - self.size.height - padding,
            },
            (ExternalSide::Bottom, ExternalAlignment::Start) => Point {
                x: rect.origin.x + padding,
                y: rect.bottom() + padding,
            },
            (ExternalSide::Bottom, ExternalAlignment::Center) => Point {
                x: rect.origin.x + (rect.size.width - self.size.width) / 2.0,
                y: rect.bottom() + padding,
            },
            (ExternalSide::Bottom, ExternalAlignment::End) => Point {
                x: rect.right() - self.size.width - padding,
                y: rect.bottom() + padding,
            },
            (ExternalSide::Left, ExternalAlignment::Start) => Point {
                x: rect.origin.x - padding - self.size.width,
                y: rect.origin.y + padding,
            },
            (ExternalSide::Left, ExternalAlignment::Center) => Point {
                x: rect.origin.x - padding - self.size.width,
                y: rect.origin.y + (rect.size.height - self.size.height) / 2.0,
            },
            (ExternalSide::Left, ExternalAlignment::End) => Point {
                x: rect.origin.x - padding - self.size.width,
                y: rect.bottom() - self.size.height - padding,
            },
        }
    }
}

impl Rect {
    /// Returns the x-coordinate of the right edge.
    pub fn right(self) -> f64 {
        self.origin.x + self.size.width
    }

    /// Returns the y-coordinate of the bottom edge.
    pub fn bottom(self) -> f64 {
        self.origin.y + self.size.height
    }

    /// Returns the rectangle's center point.
    pub fn center(self) -> Point {
        Point {
            x: self.origin.x + self.size.width / 2.0,
            y: self.origin.y + self.size.height / 2.0,
        }
    }

    /// Whether the interiors of these rectangles overlap.
    pub fn overlaps(self, other: Self) -> bool {
        self.origin.x < other.right()
            && self.right() > other.origin.x
            && self.origin.y < other.bottom()
            && self.bottom() > other.origin.y
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Preferred direction of graph flow.
pub enum Direction {
    /// Left to right.
    Right,
    /// Right to left.
    Left,
    /// Top to bottom.
    #[default]
    Down,
    /// Bottom to top.
    Up,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Fixed placement of a node relative to the final canvas.
pub enum CanvasPosition {
    /// Anchored above and to the left of the diagram.
    TopLeft,
    /// Centered along the top of the canvas.
    TopCenter,
    /// Anchored above and to the right of the diagram.
    TopRight,
    /// Centered beside the left side of the canvas.
    CenterLeft,
    /// Centered beside the right side of the canvas.
    CenterRight,
    /// Anchored below and to the left of the diagram.
    BottomLeft,
    /// Centered along the bottom of the canvas.
    BottomCenter,
    /// Anchored below and to the right of the diagram.
    BottomRight,
}

/// Authored D2 adapter attributes before measurement or container fitting.
/// Bytes preserve the original Go scalar and formatted local object identity,
/// including malformed UTF-8 used in adapter diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct D2AuthoredGeometry {
    /// Local ID in D2 key syntax, rather than the absolute graph identity.
    pub object_id: Vec<u8>,
    /// Original width scalar.
    pub width: Option<Vec<u8>>,
    /// Original height scalar.
    pub height: Option<Vec<u8>>,
    /// Original locked top coordinate.
    pub top: Option<Vec<u8>>,
    /// Original locked left coordinate.
    pub left: Option<Vec<u8>>,
    /// Serialized box before adapter fitting or position discard. This is
    /// validation input, never a placement hint or fixed-position constraint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_box: Option<D2InputBox>,
    /// Original label/style declarations, before measurement and filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<Box<D2ObjectValidation>>,
}

/// D2 style keys checked by the layout adapter, in source validation order.
/// Each pair contains the D2 spelling and its serialized graph spelling.
pub const D2_VALIDATED_STYLE_KEYS: &[(&str, &str)] = &[
    ("opacity", "opacity"),
    ("stroke-width", "strokeWidth"),
    ("font-size", "fontSize"),
    ("animated", "animated"),
    ("bold", "bold"),
    ("double-border", "doubleBorder"),
    ("filled", "filled"),
    ("italic", "italic"),
    ("multiple", "multiple"),
    ("shadow", "shadow"),
    ("3d", "3d"),
    ("underline", "underline"),
];

/// Unfiltered D2 label dimensions and style scalar bytes.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct D2ValidationAttributes {
    /// Width and height, including dimensions attached to an empty label.
    pub label_dimensions: [i64; 2],
    /// Original scalar values, keyed by the D2 spelling in validation order.
    pub style: BTreeMap<String, Vec<u8>>,
}

/// Object declarations retained for ordered D2 input validation.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct D2ObjectValidation {
    /// The wire object had an absent or null Box. A compiler-created object
    /// with no input-box snapshot is distinct from a serialized nil pointer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub missing_box: bool,
    /// Original shape spelling, before case-insensitive engine shape mapping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Vec<u8>>,
    /// Unfiltered common attributes.
    pub attributes: D2ValidationAttributes,
    /// Original optional label position, including an explicitly empty value.
    pub label_position: Option<Vec<u8>>,
    /// Original optional icon position, even without a visible icon.
    pub icon_position: Option<Vec<u8>>,
    /// First nil or unboxable near-key component. Kept separately from the
    /// canonical path so the adapter checks full depth before invalid parts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_near_key_part: Option<usize>,
}

/// Edge declarations before source-arrow reversal or label filtering.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct D2EdgeValidation {
    /// Nil entries in the original route, before reversal or discard. The
    /// parallel point slots are placeholders until ordered validation rejects
    /// the nil pointer; they must never be consumed as valid route geometry.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub nil_route_points: BTreeSet<usize>,
    /// Original absolute D2 edge name when an adapter canonicalizes endpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// Unfiltered common attributes.
    pub attributes: D2ValidationAttributes,
    /// Original optional edge-label position.
    pub label_position: Option<Vec<u8>>,
    /// Original optional percentage, even when the edge has no visible label.
    pub label_percentage: Option<f64>,
    /// Original source-arrowhead attributes, independent of arrow flags.
    pub source_arrowhead: Option<D2ValidationAttributes>,
    /// Original destination-arrowhead attributes, independent of arrow flags.
    pub target_arrowhead: Option<D2ValidationAttributes>,
    /// Original signed column index at the declared source endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_table_column: Option<i64>,
    /// Original signed column index at the declared destination endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_table_column: Option<i64>,
    /// Whether the adapter reversed endpoints after retaining these declarations.
    #[serde(default)]
    pub endpoints_reversed: bool,
}

fn d2_edge_validation_absent(values: &[Option<D2EdgeValidation>]) -> bool {
    values.iter().all(Option::is_none)
}

/// Original D2 box retained until the engine's ordered validation boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct D2InputBox {
    /// Original measured dimensions.
    pub size: Size,
    /// Original optional top-left position, also validated during fresh layout.
    pub top_left: Option<Point>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Node geometry, hierarchy, constraints, and rendered decorations.
///
/// Start with [`Node::new`] and set only the metadata represented by the input
/// diagram.
pub struct Node {
    /// Adapter-defined stable identifier used in diagnostics.
    pub external_id: String,
    /// Current rendered dimensions.
    pub size: Size,
    /// Original serialized dimensions before container fitting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_size: Option<Size>,
    /// Raw D2 geometry retained for engine-specific adapter validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d2_authored_geometry: Option<Box<D2AuthoredGeometry>>,
    /// Rendered label dimensions, when the node has a label.
    #[serde(default)]
    pub label_size: Option<Size>,
    /// Font size used when label scaling participates in layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<u32>,
    #[serde(default)]
    /// Requested or computed node-label position.
    pub label_position: LabelPosition,
    /// Parent container, or `None` for a root node.
    pub parent: Option<NodeId>,
    /// Fixed top-left coordinate.
    pub locked_position: Option<Point>,
    /// Required x-coordinate when only one axis is constrained.
    pub constrained_x: Option<f64>,
    /// Required y-coordinate when only one axis is constrained.
    pub constrained_y: Option<f64>,
    /// Node that this node should be placed near.
    pub near: Option<NodeId>,
    /// Original canonical D2 near-key components. TALA resolves these through
    /// the actual object tree, without the compiler's reserved-key aliases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d2_near_key: Option<Vec<String>>,
    /// Whether width is fixed during container and label fitting.
    pub fixed_width: bool,
    /// Whether height is fixed during container and label fitting.
    pub fixed_height: bool,
    /// Flow direction for this node's container scope.
    pub direction: Option<Direction>,
    /// Whether this node forces hierarchical placement independent of shape.
    #[serde(default)]
    pub force_hierarchy: bool,
    /// Requested minimum number of grid rows.
    pub grid_rows: Option<usize>,
    /// Requested minimum number of grid columns.
    pub grid_columns: Option<usize>,
    /// Fixed placement relative to the final canvas.
    pub canvas_position: Option<CanvasPosition>,
    /// Space reserved between a container border and its content.
    pub content_insets: Insets,
    /// Additional clearance around this node during layout.
    pub layout_margins: Insets,
    /// Label or icon rendered outside the node rectangle.
    pub external_label: Option<ExternalLabel>,
    #[serde(default)]
    /// Requested or computed icon position.
    pub icon_position: Option<LabelPosition>,
    #[serde(default)]
    /// Whether the node renders an icon.
    pub has_icon: bool,
    /// Whether grid sizing must account for child labels.
    pub label_aware_grid: bool,
    /// Whether grid children are packed recursively as one unit.
    pub packed_grid: bool,
    /// Rule used to align children inside the node.
    pub content_alignment: ContentAlignment,
    /// Additional distance between eligible edge ports.
    pub port_spread: f64,
    /// Whether to use person-shape port and label behavior.
    pub person: bool,
    #[serde(default)]
    /// Whether the node uses the D2 three-dimensional modifier.
    pub is_3d: bool,
    #[serde(default)]
    /// Whether the node uses the D2 multiple-object modifier.
    pub is_multiple: bool,
    /// Rendered shape.
    pub shape: ShapeKind,
}

impl Node {
    /// Creates an unconstrained rectangular node with standard content insets.
    pub fn new(external_id: impl Into<String>, size: Size) -> Self {
        Self {
            external_id: external_id.into(),
            size,
            declared_size: None,
            d2_authored_geometry: None,
            label_size: None,
            font_size: None,
            label_position: LabelPosition::Unset,
            parent: None,
            locked_position: None,
            constrained_x: None,
            constrained_y: None,
            near: None,
            d2_near_key: None,
            fixed_width: false,
            fixed_height: false,
            direction: None,
            force_hierarchy: false,
            grid_rows: None,
            grid_columns: None,
            canvas_position: None,
            content_insets: Insets::uniform(60.0),
            layout_margins: Insets::uniform(0.0),
            external_label: None,
            icon_position: None,
            has_icon: false,
            label_aware_grid: false,
            packed_grid: false,
            content_alignment: ContentAlignment::Padding,
            port_spread: 0.0,
            person: false,
            is_3d: false,
            is_multiple: false,
            shape: ShapeKind::Rectangle,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Directed connection between two nodes.
pub struct Edge {
    /// Source node.
    pub source: NodeId,
    /// Target node.
    pub target: NodeId,
}

/// Optional zero-based SQL-table column index at each endpoint.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EdgeTableColumns {
    /// Column selected on the source table.
    pub source: Option<usize>,
    /// Column selected on the target table.
    pub target: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
/// Whether each end of an edge renders an arrow.
pub struct EdgeArrows {
    /// Arrow at the source end.
    pub source: bool,
    /// Arrow at the target end.
    pub target: bool,
}

/// D2 arrowhead shape identity at each end of an edge.
///
/// An absent shape on an enabled arrow means D2's default arrowhead. Keeping
/// resolved names (including filled variants) is necessary for routing: TALA
/// permits shared-port routes only when their endpoint arrowhead types match.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EdgeArrowheads {
    /// Resolved source arrowhead shape, or `None` for the arrow's default.
    pub source: Option<String>,
    /// Resolved target arrowhead shape, or `None` for the arrow's default.
    pub target: Option<String>,
}

/// Original compiled D2 arrowhead shapes, before target-shape resolution.
///
/// D2's graph compiler accepts ordinary object shapes, while TALA's adapter
/// accepts only its arrowhead catalog. Keep these declarations separately
/// from [`EdgeArrowheads`] so validation can still distinguish an authored
/// rectangle from its resolved triangle fallback. Values use the graph
/// compiler's Go lowercase conversion; serialized D2 adapters preserve the
/// supplied compiled values. `Some("")` is a present default arrowhead record.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EdgeArrowheadDeclarations {
    /// Compiled source shape, or `None` when there is no D2 arrowhead record.
    pub source: Option<String>,
    /// Compiled destination shape, or `None` when there is no D2 arrowhead record.
    pub target: Option<String>,
}

fn arrowhead_declarations_absent(values: &[EdgeArrowheadDeclarations]) -> bool {
    values
        .iter()
        .all(|value| value.source.is_none() && value.target.is_none())
}

/// Parses D2's boolean scalar spellings, matching Go's `strconv.ParseBool`.
/// Other spellings, including mixed case, are invalid.
#[doc(hidden)]
pub fn parse_d2_bool(value: &str) -> Option<bool> {
    match value {
        "1" | "t" | "T" | "TRUE" | "true" | "True" => Some(true),
        "0" | "f" | "F" | "FALSE" | "false" | "False" => Some(false),
        _ => None,
    }
}

/// Resolves a D2 v0.9.0 arrowhead shape and `style.filled` into TALA's
/// effective arrowhead identity, matching `d2target.ToArrowhead`.
#[doc(hidden)]
pub fn resolve_d2_arrowhead_shape(shape: &str, filled: Option<bool>) -> String {
    match (shape, filled) {
        ("diamond", Some(true)) => "filled-diamond".into(),
        ("circle", Some(true)) => "filled-circle".into(),
        ("box", Some(true)) => "filled-box".into(),
        ("triangle", Some(false)) | ("", Some(false)) => "unfilled-triangle".into(),
        ("", _) => "triangle".into(),
        (known, _)
            if matches!(
                known,
                "none"
                    | "arrow"
                    | "triangle"
                    | "diamond"
                    | "circle"
                    | "box"
                    | "cross"
                    | "cf-one"
                    | "cf-many"
                    | "cf-one-required"
                    | "cf-many-required"
            ) =>
        {
            known.into()
        }
        (_, Some(false)) => "unfilled-triangle".into(),
        _ => "triangle".into(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
/// Text rendered next to an arrowhead.
pub struct ArrowheadLabel {
    /// Label text.
    pub text: String,
    /// Rendered dimensions.
    pub size: Size,
    /// Explicit arrowhead-label color, if supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
/// Optional labels attached to the two arrowheads of an edge.
pub struct EdgeArrowheadLabels {
    /// Source-arrow label.
    pub source: Option<ArrowheadLabel>,
    /// Target-arrow label.
    pub target: Option<ArrowheadLabel>,
}

/// Explicit D2 edge style fields consulted by TALA when deciding whether
/// multiple routes may occupy the same OVG edge.
///
/// Values remain strings because D2's serialized scalar values are strings
/// and TALA compares the explicit values rather than their rendered defaults.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EdgeStyle {
    /// Label language retained from the source block-string tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Explicit opacity value.
    pub opacity: Option<String>,
    /// Explicit stroke color.
    pub stroke: Option<String>,
    /// Explicit stroke width.
    pub stroke_width: Option<String>,
    /// Explicit dash pattern.
    pub stroke_dash: Option<String>,
    /// Explicit corner radius for routed edge bends.
    pub border_radius: Option<String>,
    /// Explicit animation value.
    pub animated: Option<String>,
    /// Explicit font color for an edge label or arrowhead label.
    pub font_color: Option<String>,
    /// Explicit font family selector (`mono` and D2 font aliases).
    pub font: Option<String>,
    /// Explicit exported italic value; Edge.Text measurement stays italic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<String>,
    /// Explicit underline value for an edge label.
    pub underline: Option<String>,
    /// Explicit text transformation for edge labels.
    pub text_transform: Option<String>,
    /// Explicit fill value used by filled edge strokes.
    pub fill: Option<String>,
    /// Whether the edge's target arrowhead is filled.
    pub filled: Option<String>,
}

/// Authored style on D2's synthetic document/board root. Layout does not
/// consult it; exporter hash and canvas paint retain the original scalars.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RootStyle {
    /// Explicit root fill scalar.
    pub fill: Option<String>,
    /// Explicit root fill-pattern scalar.
    pub fill_pattern: Option<String>,
    /// Explicit root stroke scalar.
    pub stroke: Option<String>,
    /// Explicit root stroke-width scalar.
    pub stroke_width: Option<String>,
    /// Explicit root stroke-dash scalar.
    pub stroke_dash: Option<String>,
    /// Explicit root border-radius scalar.
    pub border_radius: Option<String>,
    /// Explicit root double-border scalar.
    pub double_border: Option<String>,
    /// Explicit root opacity scalar.
    pub opacity: Option<String>,
    /// Explicit root shadow scalar.
    pub shadow: Option<String>,
    /// Explicit root three-dee scalar.
    pub three_dee: Option<String>,
    /// Explicit root multiple scalar.
    pub multiple: Option<String>,
    /// Explicit root font-color scalar.
    pub font_color: Option<String>,
    /// Explicit root font scalar.
    pub font: Option<String>,
    /// Explicit root bold scalar.
    pub bold: Option<String>,
    /// Explicit root italic scalar.
    pub italic: Option<String>,
    /// Explicit root underline scalar.
    pub underline: Option<String>,
    /// Explicit root icon-border-radius scalar.
    pub icon_border_radius: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
/// Position of a label or icon relative to its owning node or route.
///
/// Names describe two axes: the side or region first, then alignment along
/// that side. `Unlocked` values allow the engine to choose a horizontal side
/// while preserving the requested vertical band.
pub enum LabelPosition {
    /// No explicit position; the engine or adapter may choose one.
    #[default]
    Unset,
    /// Outside the top edge, aligned to its left end.
    OutsideTopLeft,
    /// Outside and centered above the top edge.
    OutsideTopCenter,
    /// Outside the top edge, aligned to its right end.
    OutsideTopRight,
    /// Outside the left edge, aligned to its top end.
    OutsideLeftTop,
    /// Outside and centered beside the left edge.
    OutsideLeftMiddle,
    /// Outside the left edge, aligned to its bottom end.
    OutsideLeftBottom,
    /// Outside the right edge, aligned to its top end.
    OutsideRightTop,
    /// Outside and centered beside the right edge.
    OutsideRightMiddle,
    /// Outside the right edge, aligned to its bottom end.
    OutsideRightBottom,
    /// Outside the bottom edge, aligned to its left end.
    OutsideBottomLeft,
    /// Outside and centered below the bottom edge.
    OutsideBottomCenter,
    /// Outside the bottom edge, aligned to its right end.
    OutsideBottomRight,
    /// Inside the top-left corner.
    InsideTopLeft,
    /// Inside and centered along the top edge.
    InsideTopCenter,
    /// Inside the top-right corner.
    InsideTopRight,
    /// Inside and centered along the left edge.
    InsideMiddleLeft,
    /// Centered inside the owner.
    InsideMiddleCenter,
    /// Inside and centered along the right edge.
    InsideMiddleRight,
    /// Inside the bottom-left corner.
    InsideBottomLeft,
    /// Inside and centered along the bottom edge.
    InsideBottomCenter,
    /// Inside the bottom-right corner.
    InsideBottomRight,
    /// On the top border near its left end.
    BorderTopLeft,
    /// Centered on the top border.
    BorderTopCenter,
    /// On the top border near its right end.
    BorderTopRight,
    /// On the left border near its top end.
    BorderLeftTop,
    /// Centered on the left border.
    BorderLeftMiddle,
    /// On the left border near its bottom end.
    BorderLeftBottom,
    /// On the right border near its top end.
    BorderRightTop,
    /// Centered on the right border.
    BorderRightMiddle,
    /// On the right border near its bottom end.
    BorderRightBottom,
    /// On the bottom border near its left end.
    BorderBottomLeft,
    /// Centered on the bottom border.
    BorderBottomCenter,
    /// On the bottom border near its right end.
    BorderBottomRight,
    /// Automatically choose a side while staying in the top band.
    UnlockedTop,
    /// Automatically choose a side while staying in the middle band.
    UnlockedMiddle,
    /// Automatically choose a side while staying in the bottom band.
    UnlockedBottom,
}

impl LabelPosition {
    /// Whether this is one of the explicit outside label bands.
    pub const fn is_outside(self) -> bool {
        matches!(
            self,
            Self::OutsideTopLeft
                | Self::OutsideTopCenter
                | Self::OutsideTopRight
                | Self::OutsideLeftTop
                | Self::OutsideLeftMiddle
                | Self::OutsideLeftBottom
                | Self::OutsideRightTop
                | Self::OutsideRightMiddle
                | Self::OutsideRightBottom
                | Self::OutsideBottomLeft
                | Self::OutsideBottomCenter
                | Self::OutsideBottomRight
        )
    }

    /// Returns the label position after reversing the direction of its route.
    ///
    /// This is D2's `label.Position.Mirrored` transformation: reversing an
    /// edge swaps both its longitudinal end and the side from which the label
    /// is viewed.
    pub const fn mirrored(self) -> Self {
        match self {
            Self::Unset => Self::Unset,
            Self::OutsideTopLeft => Self::OutsideBottomRight,
            Self::OutsideTopCenter => Self::OutsideBottomCenter,
            Self::OutsideTopRight => Self::OutsideBottomLeft,
            Self::OutsideLeftTop => Self::OutsideRightBottom,
            Self::OutsideLeftMiddle => Self::OutsideRightMiddle,
            Self::OutsideLeftBottom => Self::OutsideRightTop,
            Self::OutsideRightTop => Self::OutsideLeftBottom,
            Self::OutsideRightMiddle => Self::OutsideLeftMiddle,
            Self::OutsideRightBottom => Self::OutsideLeftTop,
            Self::OutsideBottomLeft => Self::OutsideTopRight,
            Self::OutsideBottomCenter => Self::OutsideTopCenter,
            Self::OutsideBottomRight => Self::OutsideTopLeft,
            Self::InsideTopLeft => Self::InsideBottomRight,
            Self::InsideTopCenter => Self::InsideBottomCenter,
            Self::InsideTopRight => Self::InsideBottomLeft,
            Self::InsideMiddleLeft => Self::InsideMiddleRight,
            Self::InsideMiddleCenter => Self::InsideMiddleCenter,
            Self::InsideMiddleRight => Self::InsideMiddleLeft,
            Self::InsideBottomLeft => Self::InsideTopRight,
            Self::InsideBottomCenter => Self::InsideTopCenter,
            Self::InsideBottomRight => Self::InsideTopLeft,
            Self::BorderTopLeft => Self::BorderBottomRight,
            Self::BorderTopCenter => Self::BorderBottomCenter,
            Self::BorderTopRight => Self::BorderBottomLeft,
            Self::BorderLeftTop => Self::BorderRightBottom,
            Self::BorderLeftMiddle => Self::BorderRightMiddle,
            Self::BorderLeftBottom => Self::BorderRightTop,
            Self::BorderRightTop => Self::BorderLeftBottom,
            Self::BorderRightMiddle => Self::BorderLeftMiddle,
            Self::BorderRightBottom => Self::BorderLeftTop,
            Self::BorderBottomLeft => Self::BorderTopRight,
            Self::BorderBottomCenter => Self::BorderTopCenter,
            Self::BorderBottomRight => Self::BorderTopLeft,
            Self::UnlockedTop => Self::UnlockedBottom,
            Self::UnlockedMiddle => Self::UnlockedMiddle,
            Self::UnlockedBottom => Self::UnlockedTop,
        }
    }

    /// Parses a serialized D2 position, returning [`Self::Unset`] when unknown.
    pub fn from_d2_name(value: &str) -> Self {
        let normalized = value.to_ascii_uppercase().replace('-', "_");
        match normalized.as_str() {
            // D2 accepts the short inside-label spellings in `label.near`.
            // The long INSIDE_* names remain the serialized form.
            "TOP_LEFT" => Self::InsideTopLeft,
            "TOP_CENTER" => Self::InsideTopCenter,
            "TOP_RIGHT" => Self::InsideTopRight,
            "CENTER_LEFT" => Self::InsideMiddleLeft,
            "CENTER_CENTER" => Self::InsideMiddleCenter,
            "CENTER_RIGHT" => Self::InsideMiddleRight,
            "BOTTOM_LEFT" => Self::InsideBottomLeft,
            "BOTTOM_CENTER" => Self::InsideBottomCenter,
            "BOTTOM_RIGHT" => Self::InsideBottomRight,
            "OUTSIDE_TOP_LEFT" => Self::OutsideTopLeft,
            "OUTSIDE_TOP_CENTER" => Self::OutsideTopCenter,
            "OUTSIDE_TOP_RIGHT" => Self::OutsideTopRight,
            "OUTSIDE_LEFT_TOP" => Self::OutsideLeftTop,
            "OUTSIDE_LEFT_CENTER" => Self::OutsideLeftMiddle,
            "OUTSIDE_LEFT_MIDDLE" => Self::OutsideLeftMiddle,
            "OUTSIDE_LEFT_BOTTOM" => Self::OutsideLeftBottom,
            "OUTSIDE_RIGHT_TOP" => Self::OutsideRightTop,
            "OUTSIDE_RIGHT_CENTER" => Self::OutsideRightMiddle,
            "OUTSIDE_RIGHT_MIDDLE" => Self::OutsideRightMiddle,
            "OUTSIDE_RIGHT_BOTTOM" => Self::OutsideRightBottom,
            "OUTSIDE_BOTTOM_LEFT" => Self::OutsideBottomLeft,
            "OUTSIDE_BOTTOM_CENTER" => Self::OutsideBottomCenter,
            "OUTSIDE_BOTTOM_RIGHT" => Self::OutsideBottomRight,
            "INSIDE_TOP_LEFT" => Self::InsideTopLeft,
            "INSIDE_TOP_CENTER" => Self::InsideTopCenter,
            "INSIDE_TOP_RIGHT" => Self::InsideTopRight,
            "INSIDE_MIDDLE_LEFT" => Self::InsideMiddleLeft,
            "INSIDE_MIDDLE_CENTER" => Self::InsideMiddleCenter,
            "INSIDE_MIDDLE_RIGHT" => Self::InsideMiddleRight,
            "INSIDE_BOTTOM_LEFT" => Self::InsideBottomLeft,
            "INSIDE_BOTTOM_CENTER" => Self::InsideBottomCenter,
            "INSIDE_BOTTOM_RIGHT" => Self::InsideBottomRight,
            "BORDER_TOP_LEFT" => Self::BorderTopLeft,
            "BORDER_TOP_CENTER" => Self::BorderTopCenter,
            "BORDER_TOP_RIGHT" => Self::BorderTopRight,
            "BORDER_LEFT_TOP" => Self::BorderLeftTop,
            "BORDER_LEFT_MIDDLE" => Self::BorderLeftMiddle,
            "BORDER_LEFT_BOTTOM" => Self::BorderLeftBottom,
            "BORDER_RIGHT_TOP" => Self::BorderRightTop,
            "BORDER_RIGHT_MIDDLE" => Self::BorderRightMiddle,
            "BORDER_RIGHT_BOTTOM" => Self::BorderRightBottom,
            "BORDER_BOTTOM_LEFT" => Self::BorderBottomLeft,
            "BORDER_BOTTOM_CENTER" => Self::BorderBottomCenter,
            "BORDER_BOTTOM_RIGHT" => Self::BorderBottomRight,
            "UNLOCKED_TOP" => Self::UnlockedTop,
            "UNLOCKED_MIDDLE" => Self::UnlockedMiddle,
            "UNLOCKED_BOTTOM" => Self::UnlockedBottom,
            _ => Self::Unset,
        }
    }

    /// Returns the serialized D2 name, or an empty string for [`Self::Unset`].
    pub const fn d2_name(self) -> &'static str {
        match self {
            Self::Unset => "",
            Self::OutsideTopLeft => "OUTSIDE_TOP_LEFT",
            Self::OutsideTopCenter => "OUTSIDE_TOP_CENTER",
            Self::OutsideTopRight => "OUTSIDE_TOP_RIGHT",
            Self::OutsideLeftTop => "OUTSIDE_LEFT_TOP",
            Self::OutsideLeftMiddle => "OUTSIDE_LEFT_MIDDLE",
            Self::OutsideLeftBottom => "OUTSIDE_LEFT_BOTTOM",
            Self::OutsideRightTop => "OUTSIDE_RIGHT_TOP",
            Self::OutsideRightMiddle => "OUTSIDE_RIGHT_MIDDLE",
            Self::OutsideRightBottom => "OUTSIDE_RIGHT_BOTTOM",
            Self::OutsideBottomLeft => "OUTSIDE_BOTTOM_LEFT",
            Self::OutsideBottomCenter => "OUTSIDE_BOTTOM_CENTER",
            Self::OutsideBottomRight => "OUTSIDE_BOTTOM_RIGHT",
            Self::InsideTopLeft => "INSIDE_TOP_LEFT",
            Self::InsideTopCenter => "INSIDE_TOP_CENTER",
            Self::InsideTopRight => "INSIDE_TOP_RIGHT",
            Self::InsideMiddleLeft => "INSIDE_MIDDLE_LEFT",
            Self::InsideMiddleCenter => "INSIDE_MIDDLE_CENTER",
            Self::InsideMiddleRight => "INSIDE_MIDDLE_RIGHT",
            Self::InsideBottomLeft => "INSIDE_BOTTOM_LEFT",
            Self::InsideBottomCenter => "INSIDE_BOTTOM_CENTER",
            Self::InsideBottomRight => "INSIDE_BOTTOM_RIGHT",
            Self::BorderTopLeft => "BORDER_TOP_LEFT",
            Self::BorderTopCenter => "BORDER_TOP_CENTER",
            Self::BorderTopRight => "BORDER_TOP_RIGHT",
            Self::BorderLeftTop => "BORDER_LEFT_TOP",
            Self::BorderLeftMiddle => "BORDER_LEFT_MIDDLE",
            Self::BorderLeftBottom => "BORDER_LEFT_BOTTOM",
            Self::BorderRightTop => "BORDER_RIGHT_TOP",
            Self::BorderRightMiddle => "BORDER_RIGHT_MIDDLE",
            Self::BorderRightBottom => "BORDER_RIGHT_BOTTOM",
            Self::BorderBottomLeft => "BORDER_BOTTOM_LEFT",
            Self::BorderBottomCenter => "BORDER_BOTTOM_CENTER",
            Self::BorderBottomRight => "BORDER_BOTTOM_RIGHT",
            Self::UnlockedTop => "UNLOCKED_TOP",
            Self::UnlockedMiddle => "UNLOCKED_MIDDLE",
            Self::UnlockedBottom => "UNLOCKED_BOTTOM",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
/// Label placed along an edge route.
pub struct EdgeLabel {
    /// Label text.
    pub text: String,
    /// Rendered dimensions.
    pub size: Size,
    /// Placement relative to the route.
    pub position: LabelPosition,
    /// Fractional distance from source to target in the range normally used by D2.
    pub percentage: f64,
}

/// One edge endpoint rewrite recorded while TALA collapses an aggregate.
///
/// This is diagnostic adapter metadata for serialized v0.9.0 graphjson. It is
/// kept in the shared graph so the Rust arena can replay aggregate ownership
/// without exposing Go's pointer representation in its public API.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SerializedAggregateEdgeAbduction {
    /// Serialized TALA edge identity.
    pub edge_id: i64,
    /// Source endpoint before the aggregate was formed.
    pub originally_from_node: i64,
    /// Target endpoint before the aggregate was formed.
    pub originally_to_node: i64,
    /// Source endpoint after the aggregate rewrite.
    pub current_from_node: i64,
    /// Target endpoint after the aggregate rewrite.
    pub current_to_node: i64,
}

/// Serialized cluster ownership and vessel geometry from TALA graphjson.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SerializedClusterState {
    /// Temporary vessel identity allocated by TALA.
    pub vessel_tala_id: i64,
    /// Stable member identities in TALA's graph order.
    pub member_tala_ids: Vec<i64>,
    /// Current row/column arrangement name.
    pub arrangement: String,
    /// Desired row/column arrangement name.
    pub desired_arrangement: String,
    /// Endpoint rewrites retained by the cluster.
    pub edge_abductions: Vec<SerializedAggregateEdgeAbduction>,
    /// Owning container identity, or zero for the root scope.
    pub container_tala_id: i64,
    /// Whether member dimensions are fixed.
    pub fixed_size: bool,
    /// Cluster padding in layout units.
    pub padding: f64,
    /// Temporary vessel top-left, when serialized.
    pub vessel_position: Option<Point>,
    /// Temporary vessel dimensions, when serialized.
    pub vessel_size: Size,
}

/// Serialized sequence ownership and vessel geometry from TALA graphjson.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SerializedSequenceState {
    /// Temporary vessel identity allocated by TALA.
    pub vessel_tala_id: i64,
    /// Stable member identities in TALA's graph order.
    pub member_tala_ids: Vec<i64>,
    /// Endpoint rewrites retained by the sequence.
    pub edge_abductions: Vec<SerializedAggregateEdgeAbduction>,
    /// Owning container identity, or zero for the root scope.
    pub container_tala_id: i64,
    /// Temporary vessel top-left, when serialized.
    pub vessel_position: Option<Point>,
    /// Temporary vessel dimensions, when serialized.
    pub vessel_size: Size,
}

/// Serialized hub ownership from TALA graphjson.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SerializedHubState {
    /// Stable hub identity.
    pub hub_tala_id: i64,
    /// Stable spoke identities in serialized order.
    pub spoke_tala_ids: Vec<i64>,
}

/// One node in a serialized TALA tree record.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SerializedTreeState {
    /// Root sentinel map key for this tree.
    pub root_sentinel_tala_id: i64,
    /// Tree node identity.
    pub node_tala_id: i64,
    /// Sentinel edge identity.
    pub sentinel_edge_tala_id: i64,
    /// Decoder table key, distinct from the effective ID of a temporary edge.
    pub sentinel_edge_serialized_id: Option<i64>,
    /// Upstream orientation value.
    pub orientation: i64,
    /// Child trees in source order.
    pub children: Vec<SerializedTreeState>,
}

/// Serialized hierarchy rank assignments from TALA graphjson.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SerializedHierarchyState {
    /// Node identity to rank mapping.
    pub levels: BTreeMap<i64, i64>,
}

/// Grid spacing values attached to one graph node.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GridSpacing {
    /// Shared row and column gap.
    pub grid_gap: Option<f64>,
    /// Vertical gap override.
    pub vertical_gap: Option<f64>,
    /// Horizontal gap override.
    pub horizontal_gap: Option<f64>,
}

/// A source-level legend carried alongside the measured graph.
///
/// Legends are target metadata in D2 rather than ordinary layout nodes. Keep
/// their typed compiler graph here so renderers can draw them without
/// inserting fake obstacles into TALA.
#[derive(Clone, Debug, Default)]
pub struct Legend {
    pub label: String,
    pub shapes: Vec<LegendShape>,
    pub connections: Vec<LegendConnection>,
    /// Ordinary unmeasured compiler graph; kept outside the host layout arena.
    pub source_graph: Option<Box<SourceLegendGraph>>,
}

#[derive(Clone, Debug)]
pub struct SourceLegendGraph {
    pub label: String,
    pub graph: Graph,
    pub labels: BTreeMap<NodeId, String>,
    pub styles: BTreeMap<NodeId, SourceNodeStyle>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegendShape {
    /// Identity in the retained ordinary legend compiler graph.
    pub source_node: Option<NodeId>,
    /// Explicit source target font; legend measurement does not inherit the host family.
    pub font: Option<String>,
    pub label: String,
    pub shape: ShapeKind,
    pub fill: Option<String>,
    pub stroke: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegendConnection {
    pub source_id: String,
    pub arrowheads: EdgeArrowheads,
    pub fill: Option<String>,
    pub border_radius: Option<String>,
    pub has_icon: bool,
    pub source_label: String,
    pub target_label: String,
    pub link: Option<String>,
    pub tooltip: Option<String>,
    pub font: Option<String>,
    pub label: String,
    pub source_arrow: bool,
    pub target_arrow: bool,
    pub stroke: Option<String>,
    pub stroke_width: Option<String>,
    pub stroke_dash: Option<String>,
    pub opacity: Option<String>,
    pub animated: bool,
}

/// Adapter-only TALA loop-offset map keyed by the original Go orientation numbers.
///
/// Presence, arbitrary keys and exact floating-point values are runtime state;
/// this map is deliberately omitted from the graph JSON protocol.
#[doc(hidden)]
pub type TalaLoopOffsets = BTreeMap<i64, f64>;

/// Deferred source-adapter failures, checked at their original TALA stages.
/// Native compiler graphs leave both entries absent.
#[derive(Clone, Debug, Default)]
pub struct D2BoundaryErrors {
    /// ChildrenArray traversal and graph-object set coherence, before geometry.
    pub topology: Option<String>,
    /// Canonical object/edge ID allocation, after geometry and before near keys.
    pub identity: Option<String>,
}

/// Immutable snapshot of a compiler reference (d2graph.Reference).
#[derive(Clone, Debug, serde::Serialize)]
pub struct SourceObjectReference {
    /// Complete serialized AST key, shared by references to the same KeyPath.
    /// The snapshot contains raw bytes, ranges and original component order,
    /// without retaining the compiler's thread-local mutable AST handles.
    #[serde(serialize_with = "serialize_source_reference_key")]
    pub key: std::sync::Arc<serde_json::Value>,
    /// Position of this object's component in the shared key path.
    pub key_path_index: usize,
    /// Position of the edge in the authored map key, or -1 for a field.
    pub map_key_edge_index: isize,
}

fn serialize_source_reference_key<S: serde::Serializer>(
    key: &std::sync::Arc<serde_json::Value>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serde::Serialize::serialize(key.as_ref(), serializer)
}

/// Source object attributes with deferred reference serialization.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SourceObject {
    /// Attributes and other object members, excluding references.
    #[serde(flatten)]
    pub value: serde_json::Value,
    /// References in original compiler order, expanded only for serialization.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<SourceObjectReference>,
}

impl SourceObject {
    /// Materialize the full original object only at an inspection boundary.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("source object contains only JSON values")
    }
}

/// Immutable compiler projection shared across temporary layout graph clones.
#[derive(Clone, Debug, Default)]
pub struct SourceGraphMetadata {
    /// Structured root conversion allocated and then removed member objects.
    pub objects_allocated: bool,
    /// Source object attributes and IR references by physical arena identity.
    pub objects: BTreeMap<NodeId, SourceObject>,
    /// Source connection attributes and IR references by stable edge identity.
    pub edges: BTreeMap<EdgeId, serde_json::Value>,
}

/// Caller-owned settings for the bundled layout adapters. These correspond to
/// the resolver's hydrated options, rather than authored d2-config data.
#[derive(Clone, Debug, Default)]
pub struct LayoutEngineOptions {
    /// Dagre horizontal node separation.
    pub dagre_node_sep: Option<i64>,
    /// Dagre parallel edge separation.
    pub dagre_edge_sep: Option<i64>,
    /// ELK algorithm name.
    pub elk_algorithm: Option<String>,
    /// ELK separation between nodes in adjacent layers.
    pub elk_node_spacing: Option<i64>,
    /// ELK separation between edges and adjacent-layer nodes.
    pub elk_edge_node_spacing: Option<i64>,
    /// ELK separation between edges routed between the same pair of layers.
    pub elk_edge_edge_spacing: Option<i64>,
    /// ELK self-loop clearance.
    pub elk_self_loop_spacing: Option<i64>,
    /// ELK container padding expression, interpreted by the source adapter.
    pub elk_padding: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
/// Mutable measured graph consumed by layout engines.
///
/// Node and edge IDs are stable insertion indices. IDs returned by
/// [`Graph::add_node`] and [`Graph::add_edge`] remain valid for the lifetime of
/// the graph. Because the ID tuple fields are public for serialization,
/// callers accepting arbitrary IDs should validate them with [`Graph::node`]
/// or the corresponding iterator before calling an indexed setter.
pub struct Graph {
    /// Hydrated in-process adapter options. Graph projections clone these so
    /// nested core invocations retain their resolver settings. They do not
    /// participate in source graph JSON or the target's authored config hash.
    #[serde(skip)]
    pub layout_engine_options: LayoutEngineOptions,
    /// Routes supplied by a public target are already exported. Renderers
    /// must not apply the layout-to-target decimal/f32 conversion again.
    /// Authored compiler/layout graphs leave this false.
    #[serde(skip)]
    pub target_routes_exported: bool,
    /// Source-facing compiler records; layout clones share immutable contents.
    #[serde(skip)]
    pub source_metadata: Option<std::sync::Arc<SourceGraphMetadata>>,
    /// Compiled d2graph root attributes/structured members, outside the
    /// physical node arena and independent of target/render projections.
    #[serde(skip)]
    pub source_root: Option<serde_json::Value>,
    /// Synthetic source root metadata. Source structured roots can own edges
    /// after their children have been converted into fields/columns. This
    /// root is deliberately excluded from physical object iteration/layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_object: Option<Box<Node>>,
    /// Effective source theme for target export defaults, independent of explicit styles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_theme_id: Option<i64>,
    /// Source class-name arrays on authored objects and connections.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub target_node_classes: BTreeMap<NodeId, Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub target_edge_classes: BTreeMap<EdgeId, Vec<String>>,

    /// Source graph reconstruction failures retained for ordered validation.
    #[serde(skip)]
    pub d2_boundary_errors: D2BoundaryErrors,
    #[doc(hidden)]
    pub nodes: Vec<Node>,
    #[doc(hidden)]
    pub edges: Vec<Edge>,
    /// D2 Edge.Index, assigned in compilation order before AST edge sorting.
    /// Missing entries use insertion order for graphs built through add_edge.
    #[serde(default, skip_serializing_if = "edge_indices_absent")]
    pub edge_indices: Vec<Option<usize>>,
    /// Compiled `Graph.Data["tala-seeds"]`, including a deferred parse error.
    /// Only a TALA core layout invocation consumes this entry. Empty boards
    /// and grid/sequence scopes can complete without reading it. Cloning a
    /// projected graph preserves it for any nested core layout invocation.
    #[serde(skip)]
    pub tala_seed_data: Option<Result<Vec<i64>, String>>,
    /// Original config data, retained independently of engine seed parsing.
    /// D2 includes these uncoerced values in its target configuration hash.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub d2_config_data: BTreeMap<String, serde_json::Value>,
    /// Authored config pointers retained for the target's canonical byte stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d2_config_pad: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d2_config_center: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d2_config_layout_engine: Option<String>,
    /// Optional source legend metadata. It is intentionally excluded from
    /// graph JSON because D2 serializes legends on the target diagram, not in
    /// its measured graph.
    #[serde(skip)]
    pub legend: Option<Legend>,
    /// Source node-label text retained for layout adapters that serialize
    /// labels into an engine's wire graph. It is adapter metadata rather than
    /// part of the public measured-graph JSON contract.
    #[serde(skip)]
    pub node_labels: BTreeMap<NodeId, String>,
    /// Exact source bytes after label text transforms. Presentation strings
    /// cannot retain malformed Go strings produced by block indentation.
    #[serde(skip)]
    pub node_label_bytes: BTreeMap<NodeId, Vec<u8>>,
    /// Internal TALA identity overrides for temporary placement vessels.
    ///
    /// Serialized D2 nodes derive their identity from `external_id`, but
    /// AddSequence/AddCluster allocate vessels with a runtime `rand.Int63`
    /// ID. Keep that recovered identity alongside the adapter-only temporary
    /// node instead of replacing it with a hash of a synthetic external ID.
    #[serde(skip)]
    pub tala_id_overrides: Vec<Option<i64>>,
    /// Source TALA edge identities, separate from dense arena indexes.
    /// Unset edges use the upstream NewEdge zero identity.
    #[serde(skip)]
    #[doc(hidden)]
    pub tala_edge_ids: Vec<Option<i64>>,
    /// Explicit TALA edge visibility state for reconstructed or projected edges.
    /// None asks the adapter to derive the initial flag from the edge style.
    #[serde(skip)]
    #[doc(hidden)]
    pub tala_edge_invisible: Vec<Option<bool>>,
    /// Original graphjson endpoint identities retained across TALA's
    /// carrier rewrites. This is adapter metadata used by diagnostic parity
    /// traces and is empty for programmatic graphs.
    #[serde(skip)]
    #[doc(hidden)]
    pub source_edge_endpoints: Vec<(i64, i64)>,
    /// Endpoint lookup keyed by serialized TALA edge identity so the map
    /// remains valid after preprocessing adds or reorders carrier edges.
    #[serde(skip)]
    #[doc(hidden)]
    pub source_edge_endpoints_by_id: BTreeMap<i64, (i64, i64)>,
    /// Explicit upstream Label.FixPosition state, independent of position.
    #[serde(skip)]
    #[doc(hidden)]
    pub tala_edge_label_fixed: Vec<bool>,
    /// Direct-engine label positions retained for main and arrowhead labels.
    #[serde(skip)]
    #[doc(hidden)]
    pub tala_edge_label_positions: BTreeMap<EdgeId, [Option<LabelPosition>; 3]>,
    /// Explicit sibling order for each container; `None` is the root scope.
    #[serde(skip)]
    pub hierarchy_children_order: BTreeMap<Option<NodeId>, Vec<NodeId>>,
    /// Optional serialized order of the owning engine's node slice.
    ///
    /// Most callers derive order from hierarchy children. The diagnostic
    /// graphjson codec must preserve D2's `Graph.Nodes` order independently
    /// from its map-backed container member order because the former feeds
    /// order-sensitive optimizer shuffles.
    #[serde(skip)]
    #[doc(hidden)]
    pub node_order_override: Vec<NodeId>,
    /// Exact GraphJSON graph members, separate from the complete interned
    /// record inventory. Some(empty, empty) is an explicitly empty graph.
    /// This decoder-to-engine transfer is consumed before layout starts.
    #[serde(skip)]
    #[doc(hidden)]
    pub tala_decoded_members: Option<(Vec<NodeId>, Vec<EdgeId>)>,
    /// Map-only GraphJSON vessels retain a genuine nil Node.Graph field.
    /// Record existence is independent of graph membership and ownership.
    #[serde(skip)]
    #[doc(hidden)]
    pub tala_decoded_nil_owners: BTreeSet<NodeId>,
    /// GraphJSON edge table keys retain pointer identity when multiple
    /// temporary edges share the effective ID zero.
    #[serde(skip)]
    #[doc(hidden)]
    pub tala_decoded_edge_records: BTreeMap<i64, EdgeId>,
    /// Temporary placement graphs preserve TALA's Node.isContainer bit even
    /// after copied nodes are detached from the owning hierarchy. Ordinary
    /// serialized graphs leave this empty and ArenaGraph derives the bit from
    /// parent/child relationships as before.
    #[serde(skip)]
    pub container_flags: Vec<bool>,
    /// Incident edges that remain on a shared Go `*Node` while a temporary
    /// children graph omits the corresponding external `Graph.Edges` entry.
    ///
    /// TALA's `CopyEntitiesFrom` aliases node pointers: tree extraction sees
    /// those external edges when testing `len(Node.Edges)`, even though the
    /// temporary graph only optimizes projected internal edges.  Rust clones
    /// the nodes, so retain this small piece of pointer-visible topology as
    /// adapter metadata rather than inventing routable shadow edges.
    #[serde(skip)]
    pub external_edge_counts: Vec<usize>,
    /// Adapter-restored TALA `Node.IsInvisible` bits. D2 derives this from
    /// opacity and transparent fill/stroke rather than from node geometry.
    #[serde(skip)]
    #[doc(hidden)]
    pub node_invisible: Vec<bool>,
    /// Whether the source object declared a label language. Dagre uses this
    /// target metadata when reproducing D2's default icon placement for
    /// language-rendered labels.
    #[serde(skip)]
    #[doc(hidden)]
    pub node_has_language: Vec<bool>,
    #[serde(default)]
    #[doc(hidden)]
    pub edge_arrows: Vec<EdgeArrows>,
    #[serde(default)]
    #[doc(hidden)]
    pub edge_arrowheads: Vec<EdgeArrowheads>,
    /// Compiled D2 shape records retained across adapters and graph JSON.
    /// Resolved TALA graphjson input leaves this metadata absent.
    #[serde(default, skip_serializing_if = "arrowhead_declarations_absent")]
    #[doc(hidden)]
    pub edge_arrowhead_declarations: Vec<EdgeArrowheadDeclarations>,
    #[serde(default)]
    #[doc(hidden)]
    pub edge_labels: Vec<Option<EdgeLabel>>,
    /// Presence of the D2 label-percentage pointer. Missing percentages and
    /// explicit zero have different source-arrow reversal semantics.
    /// None preserves legacy numeric-only callers as explicitly specified.
    #[serde(default)]
    #[doc(hidden)]
    pub edge_label_percentage_presence: Vec<Option<bool>>,
    /// Effective font size used to render each edge label. D2 keeps this
    /// outside the GraphJSON label record, so it remains adapter metadata.
    #[serde(skip)]
    #[doc(hidden)]
    pub edge_font_sizes: Vec<u32>,
    /// Whether each edge label uses D2's explicit `style.bold` face.
    #[serde(skip)]
    #[doc(hidden)]
    pub edge_label_bold: Vec<bool>,
    #[serde(default)]
    #[doc(hidden)]
    pub edge_arrowhead_labels: Vec<EdgeArrowheadLabels>,
    #[serde(default)]
    #[doc(hidden)]
    pub edge_styles: Vec<EdgeStyle>,
    /// Optional hyperlinks attached to edge routes.
    #[serde(default)]
    #[doc(hidden)]
    pub edge_links: Vec<Option<String>>,
    /// Optional tooltips attached to edge routes.
    #[serde(default)]
    #[doc(hidden)]
    pub edge_tooltips: Vec<Option<String>>,
    /// Optional icons attached to edge routes. These are target decorations,
    /// kept separate from [`EdgeStyle`] because D2 serializes them directly
    /// on `Connection` alongside label metadata.
    #[serde(skip)]
    #[doc(hidden)]
    pub edge_icons: Vec<Option<String>>,
    /// Requested position for each edge icon.
    #[serde(skip)]
    #[doc(hidden)]
    pub edge_icon_positions: Vec<Option<LabelPosition>>,
    /// Optional rounded clipping radius for each edge icon.
    #[serde(skip)]
    #[doc(hidden)]
    pub edge_icon_border_radii: Vec<f64>,
    /// Whether each edge route is a cubic curve rather than an orthogonal
    /// polyline. This mirrors GraphJSON's `isCurve` bit so renderers can
    /// preserve Dagre and explicitly curved routes across the graph boundary.
    #[serde(default)]
    #[doc(hidden)]
    pub edge_is_curves: Vec<bool>,
    #[serde(default)]
    #[doc(hidden)]
    pub edge_table_columns: Vec<EdgeTableColumns>,
    /// Existing serialized routes used by D2's standalone `routeedges`
    /// entrypoint. Ordinary layout inputs leave these empty.
    #[serde(skip)]
    #[doc(hidden)]
    pub edge_routes: Vec<Vec<Point>>,
    /// Original D2 routes before layout discard or source-arrow reversal.
    /// Absent entries validate the working route. These never seed placement.
    #[serde(skip)]
    #[doc(hidden)]
    pub d2_input_routes: Vec<Option<Vec<Point>>>,
    /// Original D2 attributes before edge canonicalization and label filtering.
    #[serde(default, skip_serializing_if = "d2_edge_validation_absent")]
    #[doc(hidden)]
    pub d2_edge_validation: Vec<Option<D2EdgeValidation>>,
    /// Optional serialized layout-engine minimum dimensions for each edge.
    /// When absent, adapters derive dimensions from labels and arrowheads.
    #[serde(skip)]
    #[doc(hidden)]
    pub edge_min_widths: Vec<Option<f64>>,
    #[serde(skip)]
    #[doc(hidden)]
    pub edge_min_heights: Vec<Option<f64>>,
    /// Number of rendered SQL-table rows, excluding the table header.
    ///
    /// This is graph metadata rather than a `Node` field so existing public
    /// node literals remain source compatible. TALA uses the count to create
    /// one temporary hierarchy placement node for every table column.
    #[serde(default)]
    #[doc(hidden)]
    pub table_column_counts: Vec<Option<usize>>,
    /// Source SQL column labels used as ELK port identities, in row order.
    #[doc(hidden)]
    #[serde(skip)]
    pub table_column_names: BTreeMap<NodeId, Vec<String>>,
    /// Adapter-only positions from a serialized layout snapshot. These are
    /// hints for order-sensitive preprocessing such as TALA cluster vessel
    /// creation; they never constrain ordinary placement.
    #[serde(skip)]
    #[doc(hidden)]
    pub initial_positions: Vec<Option<Point>>,
    /// Live TALA loop maps retained by temporary placement projections.
    /// Missing entries are nil, while an explicit empty map remains nonnil.
    #[serde(skip)]
    #[doc(hidden)]
    pub tala_node_loop_offsets: Vec<Option<TalaLoopOffsets>>,
    /// Exact desired dimensions decoded from TALA GraphJSON, independent of
    /// current measured boxes. Missing keys retain ordinary adapter behavior.
    #[doc(hidden)]
    #[serde(skip)]
    pub tala_desired_dimensions: BTreeMap<NodeId, [Option<f64>; 2]>,
    /// Exact nullable Label presence from TALA GraphJSON. Decoded layout nodes
    /// already carry labels independently of D2 Object.HasLabel adapter rules.
    /// Missing entries retain ordinary object-to-layout translation behavior.
    #[doc(hidden)]
    #[serde(skip)]
    pub tala_decoded_label_presence: BTreeMap<NodeId, bool>,
    /// Nullable GraphJSON route records consumed by the TALA owning arena.
    /// Public numerical routes remain the adapter view; an explicit setter
    /// invalidates this source record before supplying a replacement route.
    #[doc(hidden)]
    #[serde(skip)]
    pub tala_decoded_routes: BTreeMap<EdgeId, Vec<Option<Point>>>,
    /// Source grid spacing for each node. This remains adapter metadata so
    /// graph JSON stays compatible while nested grid orchestration is built.
    #[doc(hidden)]
    #[serde(skip)]
    pub grid_spacing: Vec<GridSpacing>,
    /// Whether each grid scope fills objects left-to-right by row.
    ///
    /// D2 preserves the source order of `grid-rows` and `grid-columns` when
    /// both are present: rows first means row-directed placement, columns
    /// first means column-directed placement. `None` keeps the historical
    /// row-directed default for programmatically constructed graphs.
    #[doc(hidden)]
    #[serde(skip)]
    pub grid_row_directed: Vec<Option<bool>>,
    /// Effective root flow direction.
    pub direction: Direction,
    /// The direction explicitly supplied by the diagram, if any. `direction`
    /// is the effective layout fallback, while scoring must preserve an absent
    /// root direction as `NONE`.
    #[serde(default)]
    pub explicit_direction: Option<Direction>,
    /// Root-scope grid row count from the source document, when specified.
    ///
    /// The graph JSON model has no synthetic root object, so root-level grid
    /// properties need a separate adapter field to survive lowering.
    #[doc(hidden)]
    #[serde(skip)]
    pub root_grid_rows: Option<usize>,
    /// Root-scope grid column count from the source document, when specified.
    #[doc(hidden)]
    #[serde(skip)]
    pub root_grid_columns: Option<usize>,
    /// Root-scope grid gap, in source layout units.
    #[doc(hidden)]
    #[serde(skip)]
    pub root_grid_gap: Option<f64>,
    /// Root-scope vertical grid gap override.
    #[doc(hidden)]
    #[serde(skip)]
    pub root_vertical_gap: Option<f64>,
    /// Root-scope horizontal grid gap override.
    #[doc(hidden)]
    #[serde(skip)]
    pub root_horizontal_gap: Option<f64>,
    /// Source ordering of root-level grid rows and columns.
    #[doc(hidden)]
    #[serde(skip)]
    pub root_grid_row_directed: Option<bool>,
    /// Whether the root scope forces hierarchical placement.
    #[serde(default)]
    pub root_hierarchy: bool,
    /// Whether the serialized document root uses the `sequence_diagram` shape.
    ///
    /// D2 represents the document root as a synthetic object, so this marker
    /// cannot be attached to a regular [`Node`]. It is kept separate from
    /// `sequence_diagram_nodes` for source layouts whose root has no object
    /// record in graph JSON.
    #[serde(default, skip_serializing_if = "is_false")]
    pub root_sequence_diagram: bool,
    /// Style of the synthetic document/selected board root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_style: Option<RootStyle>,
    /// Additional symmetric proximity relations decoded from D2 graphjson.
    ///
    /// The public diagram model exposes the common one-to-one `near` field;
    /// TALA's graphjson wire format permits a node to list several peers. The
    /// direct engine adapter keeps those extra relations here so the Rust
    /// arena can preserve the upstream proximity graph without changing the
    /// ordinary public object protocol.
    #[doc(hidden)]
    #[serde(skip)]
    pub near_targets: Vec<Vec<NodeId>>,
    /// Serialized cluster ownership retained by the diagnostic graphjson
    /// adapter. Ordinary programmatic graphs leave this empty.
    #[doc(hidden)]
    #[serde(skip)]
    pub serialized_clusters: Vec<SerializedClusterState>,
    /// Serialized sequence ownership retained by the diagnostic graphjson
    /// adapter. Ordinary programmatic graphs leave this empty.
    #[doc(hidden)]
    #[serde(skip)]
    pub serialized_sequences: Vec<SerializedSequenceState>,
    /// Serialized hub ownership retained by the diagnostic graphjson adapter.
    #[doc(hidden)]
    #[serde(skip)]
    pub serialized_hubs: Vec<SerializedHubState>,
    /// Serialized tree ownership retained by the diagnostic graphjson adapter.
    #[doc(hidden)]
    #[serde(skip)]
    pub serialized_trees: BTreeMap<i64, Vec<SerializedTreeState>>,
    /// Serialized hierarchy ranks retained by the diagnostic graphjson adapter.
    #[doc(hidden)]
    #[serde(skip)]
    pub serialized_hierarchies: Vec<SerializedHierarchyState>,
    /// Serialized per-container flow directions retained by the diagnostic
    /// graphjson adapter, including key zero for the root direction.
    #[doc(hidden)]
    #[serde(skip)]
    pub serialized_directions: BTreeMap<i64, Direction>,
    /// Cell size supplied by a serialized graphjson snapshot, when present.
    #[doc(hidden)]
    #[serde(skip)]
    pub serialized_cell_size: Option<f64>,
    /// Routing costs supplied by a serialized graphjson snapshot.
    #[doc(hidden)]
    #[serde(skip)]
    pub serialized_routing_costs: Option<(f64, f64, f64)>,
    /// Adapter-only marker for nodes whose source shape is
    /// `sequence_diagram`. Keeping this separate from [`ShapeKind`] preserves
    /// TALA's ordinary shape vocabulary while the sequence engine owns its
    /// specialized actor/message semantics.
    #[doc(hidden)]
    #[serde(skip)]
    pub sequence_diagram_nodes: Vec<bool>,
    /// Source declaration/reference scope ownership retained for sequence
    /// layout and target classification. Outward-resolved edges can belong to
    /// a different scope from their canonical IR owner. Endpoint ancestry
    /// alone cannot recover which container declared each message.
    #[doc(hidden)]
    #[serde(skip)]
    pub sequence_group_edges: BTreeMap<NodeId, Vec<EdgeId>>,
    /// Earliest non-glob MapKey line from original object/edge references.
    /// Sequence vertical ordering compares source lines, never arena IDs.
    /// Absent references use Go's MaxInt32 sentinel at the layout boundary.
    #[serde(skip)]
    pub sequence_node_lines: BTreeMap<NodeId, i64>,
    #[serde(skip)]
    pub sequence_edge_lines: BTreeMap<EdgeId, i64>,
    /// Original Object.hasEdgeRef, derived from reference MapKey.Edges.
    /// Explicit false distinguishes glob-created endpoints from declarations.
    #[serde(skip)]
    pub sequence_node_edge_refs: BTreeMap<NodeId, bool>,
    /// Canonical graph scopes of original object declaration references.
    /// Scope ancestry, independently of child ancestry, owns contained objects.
    #[serde(skip)]
    pub sequence_object_scopes: BTreeMap<NodeId, Vec<NodeId>>,
    /// Non-glob object reference scopes used by sequence group note bounds.
    #[serde(skip)]
    pub sequence_note_scopes: BTreeMap<NodeId, Vec<NodeId>>,
    /// Original font declaration presence, distinct from a measured default.
    #[serde(skip)]
    pub node_font_size_declarations: BTreeMap<NodeId, Option<u32>>,
    /// Original Object.Level retained while extracting temporary graph roots.
    #[serde(skip)]
    pub node_label_levels: BTreeMap<NodeId, u32>,
    /// Source-frame positions retained across temporary layout graphs for
    /// parent-relative projected edge scoring. This adapter metadata is
    /// intentionally omitted from serialized public graph formats.
    #[doc(hidden)]
    #[serde(skip)]
    pub source_positions_by_tala: BTreeMap<i64, Point>,
    /// Source parent identities paired with [`Graph::source_positions_by_tala`].
    #[doc(hidden)]
    #[serde(skip)]
    pub source_parent_tala_by_tala: BTreeMap<i64, Option<i64>>,
}

fn is_false(value: &bool) -> bool {
    !value
}

impl Graph {
    /// Resolves ordinary Object.Text fonts at the TALA adapter boundary.
    /// Extracted roots preserve original levels, while sequence ancestry can
    /// change. Cached label dimensions remain the original SetDimensions data.
    #[doc(hidden)]
    pub fn tala_text_font_size(&self, id: NodeId) -> Option<u32> {
        let node = self.node(id)?;
        let Some(declared) = self.node_font_size_declarations.get(&id) else {
            return node.font_size;
        };
        // Go's adapter takes Object.Text.FontSize, then removes HeaderFontAdd
        // for classes/tables. Source nodes retain their measured header size
        // when no scalar was declared, so use the base20 here.
        if matches!(node.shape, ShapeKind::Class | ShapeKind::SqlTable) {
            return Some(declared.unwrap_or(20));
        }
        let mut parent = node.parent;
        let mut in_sequence = self.root_sequence_diagram;
        while let Some(candidate) = parent {
            if self.is_sequence_diagram(candidate) {
                in_sequence = true;
                break;
            }
            parent = self.node(candidate).and_then(|node| node.parent);
        }
        let container = self.nodes().any(|(_, child)| child.parent == Some(id))
            || node.grid_rows.is_some()
            || node.grid_columns.is_some();
        let default = if !in_sequence && container && node.shape != ShapeKind::Text {
            match self.node_label_levels.get(&id).copied().unwrap_or(1) {
                1 => 28,
                2 => 24,
                3 => 20,
                _ => 16,
            }
        } else {
            16
        };
        Some(declared.unwrap_or(default))
    }

    /// Whether original object reference scopes descend from this object.
    /// Legacy measured graphs without reference metadata retain child ownership.
    pub fn sequence_contains_object(&self, id: NodeId) -> bool {
        if self.sequence_object_scopes.is_empty() {
            return self.nodes.iter().any(|node| node.parent == Some(id));
        }
        self.sequence_object_scopes.values().flatten().any(|scope| {
            let mut current = Some(*scope);
            while let Some(candidate) = current {
                if candidate == id {
                    return true;
                }
                current = self.node(candidate).and_then(|node| node.parent);
            }
            false
        })
    }

    /// Source reference state for compiled objects. Legacy measured graphs
    /// without declaration metadata retain their explicit-edge interpretation.
    /// Such graphs do not qualify the full original reference API contract.
    pub fn sequence_has_edge_reference(&self, id: NodeId) -> bool {
        self.sequence_node_edge_refs
            .get(&id)
            .copied()
            .unwrap_or_else(|| {
                self.edges
                    .iter()
                    .any(|edge| edge.source == id || edge.target == id)
            })
    }

    /// Creates an empty graph with an explicit root flow direction.
    pub fn with_direction(direction: Direction) -> Self {
        Self {
            direction,
            explicit_direction: Some(direction),
            ..Self::default()
        }
    }

    /// Sets the source hierarchy's sibling order.
    ///
    /// Keys are parent IDs; `None` identifies root nodes. Child IDs not present
    /// in the graph are rejected later by layout validation.
    pub fn set_hierarchy_children_order(&mut self, order: BTreeMap<Option<NodeId>, Vec<NodeId>>) {
        self.hierarchy_children_order = order;
    }

    /// Sets the owning layout graph's serialized node order.
    ///
    /// This is an adapter-only hint. Unknown or repeated IDs are ignored by
    /// the engine's conversion boundary, which appends any omitted nodes in
    /// its normal deterministic order.
    pub fn set_node_order_override(&mut self, order: Vec<NodeId>) {
        self.node_order_override = order;
    }

    /// Appends a node and returns its stable ID.
    pub fn add_node(&mut self, node: Node) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(node);
        self.tala_id_overrides.push(None);
        self.container_flags.push(false);
        self.external_edge_counts.push(0);
        self.node_invisible.push(false);
        self.node_has_language.push(false);
        self.table_column_counts.push(None);
        self.initial_positions.push(None);
        self.tala_node_loop_offsets.resize(id.0 as usize, None);
        self.tala_node_loop_offsets.push(None);
        self.grid_spacing.push(GridSpacing::default());
        self.grid_row_directed.push(None);
        self.near_targets.push(Vec::new());
        self.sequence_diagram_nodes.push(false);
        id
    }

    /// Records the source label text for a node without changing its measured
    /// dimensions or public graph serialization.
    pub fn set_node_label(&mut self, id: NodeId, label: impl Into<String>) {
        let label = label.into();
        self.node_label_bytes.insert(id, label.as_bytes().to_vec());
        self.node_labels.insert(id, label);
    }

    /// Records a Go label without replacing invalid bytes. The text map is a
    /// presentation for measured-layout adapters; scene checks use raw bytes.
    pub fn set_node_label_bytes(&mut self, id: NodeId, bytes: Vec<u8>) {
        let text = ast::ByteString::from(bytes.as_slice())
            .to_text()
            .into_owned();
        self.node_labels.insert(id, text);
        self.node_label_bytes.insert(id, bytes);
    }

    /// Marks whether a node is the root of a sequence-diagram scope.
    pub fn set_sequence_diagram(&mut self, id: NodeId, is_sequence_diagram: bool) {
        if self.sequence_diagram_nodes.len() < self.nodes.len() {
            self.sequence_diagram_nodes.resize(self.nodes.len(), false);
        }
        if let Some(value) = self.sequence_diagram_nodes.get_mut(id.0 as usize) {
            *value = is_sequence_diagram;
        }
    }

    /// Returns whether a node is marked as a sequence-diagram scope.
    pub fn is_sequence_diagram(&self, id: NodeId) -> bool {
        self.sequence_diagram_nodes
            .get(id.0 as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Records a serialized node position as a preprocessing hint without
    /// locking the node to that coordinate during layout.
    ///
    /// # Panics
    ///
    /// Panics when `id` was not returned by [`Graph::add_node`] for `graph`.
    pub fn set_initial_position(&mut self, id: NodeId, position: Option<Point>) {
        if self.initial_positions.len() <= id.0 as usize {
            self.initial_positions.resize(self.nodes.len(), None);
        }
        self.initial_positions[id.0 as usize] = position;
    }

    /// Stores source grid spacing for a node.
    pub fn set_grid_spacing(&mut self, id: NodeId, spacing: GridSpacing) {
        if self.grid_spacing.len() < self.nodes.len() {
            self.grid_spacing
                .resize(self.nodes.len(), GridSpacing::default());
        }
        self.grid_spacing[id.0 as usize] = spacing;
    }

    /// Records whether a node's grid is filled left-to-right by row.
    pub fn set_grid_row_directed(&mut self, id: NodeId, row_directed: Option<bool>) {
        if self.grid_row_directed.len() < self.nodes.len() {
            self.grid_row_directed.resize(self.nodes.len(), None);
        }
        self.grid_row_directed[id.0 as usize] = row_directed;
    }

    /// Returns source grid spacing for a node, or defaults when absent.
    #[must_use]
    pub fn grid_spacing(&self, id: NodeId) -> GridSpacing {
        self.grid_spacing
            .get(id.0 as usize)
            .copied()
            .unwrap_or_default()
    }

    /// Marks whether a node is visually absent but still participates in routing.
    ///
    /// An invisible node retains geometry and ports for obstacle and topology
    /// calculations, but visibility-graph construction does not treat its
    /// center as an ordinary visible node center.
    ///
    /// # Panics
    ///
    /// Panics when `id` was not returned by [`Graph::add_node`] for this graph.
    pub fn set_node_invisible(&mut self, id: NodeId, is_invisible: bool) {
        if self.node_invisible.len() <= id.0 as usize {
            self.node_invisible.resize(self.nodes.len(), false);
        }
        self.node_invisible[id.0 as usize] = is_invisible;
    }

    /// Returns whether a node is marked invisible.
    pub fn node_is_invisible(&self, id: NodeId) -> bool {
        self.node_invisible
            .get(id.0 as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Records whether a node's label uses D2's language-specific renderer.
    pub fn set_node_has_language(&mut self, id: NodeId, has_language: bool) {
        if self.node_has_language.len() < self.nodes.len() {
            self.node_has_language.resize(self.nodes.len(), false);
        }
        if let Some(value) = self.node_has_language.get_mut(id.0 as usize) {
            *value = has_language;
        }
    }

    /// Returns whether a node's source label specified a language.
    #[must_use]
    pub fn node_has_language(&self, id: NodeId) -> bool {
        self.node_has_language
            .get(id.0 as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Sets the rendered SQL-table row count used to derive table ports.
    ///
    /// `count` excludes the header compartment. Use `None` for a non-table
    /// node or when the adapter has no serialized row inventory.
    ///
    /// # Panics
    ///
    /// Panics when `id` was not returned by [`Graph::add_node`] for this graph.
    pub fn set_table_column_count(&mut self, id: NodeId, count: Option<usize>) {
        if self.table_column_counts.len() <= id.0 as usize {
            self.table_column_counts.resize(self.nodes.len(), None);
        }
        self.table_column_counts[id.0 as usize] = count;
    }

    /// Returns the rendered SQL-table row count for a node.
    pub fn table_column_count(&self, id: NodeId) -> Option<usize> {
        self.table_column_counts
            .get(id.0 as usize)
            .copied()
            .flatten()
    }

    pub fn set_container_flag(&mut self, id: NodeId, is_container: bool) {
        if self.container_flags.len() <= id.0 as usize {
            self.container_flags.resize(self.nodes.len(), false);
        }
        self.container_flags[id.0 as usize] = is_container;
    }

    /// Whether endpoints and their attached arrows identify equivalent edges.
    pub fn edges_equivalent(&self, left: EdgeId, right: EdgeId) -> bool {
        let (Some(a), Some(b)) = (
            self.edges.get(left.0 as usize),
            self.edges.get(right.0 as usize),
        ) else {
            return false;
        };
        let (aa, ba) = (self.edge_arrows(left), self.edge_arrows(right));
        (a.source == b.source && a.target == b.target && aa == ba)
            || (a.source == b.target
                && a.target == b.source
                && aa.source == ba.target
                && aa.target == ba.source)
    }

    /// D2's equivalent-edge index, independent of this graph's array position.
    pub fn edge_index(&self, id: EdgeId) -> Option<usize> {
        self.edges.get(id.0 as usize)?;
        Some(
            self.edge_indices
                .get(id.0 as usize)
                .copied()
                .flatten()
                .unwrap_or_else(|| {
                    self.edges()
                        .take(id.0 as usize)
                        .filter(|(previous, _)| self.edges_equivalent(*previous, id))
                        .count()
                }),
        )
    }

    /// Retains an index assigned before graph sorting or projection.
    pub fn set_edge_index(&mut self, id: EdgeId, index: usize) {
        if self.edge_indices.len() <= id.0 as usize {
            self.edge_indices.resize(self.edges.len(), None);
        }
        if let Some(value) = self.edge_indices.get_mut(id.0 as usize) {
            *value = Some(index);
        }
    }

    /// Appends an edge and returns its stable ID.
    pub fn add_edge(&mut self, edge: Edge) -> EdgeId {
        let id = EdgeId(self.edges.len() as u32);
        self.edges.push(edge);
        self.tala_edge_ids.push(None);
        self.tala_edge_invisible.push(None);
        self.tala_edge_label_fixed.push(false);
        self.edge_arrows.push(EdgeArrows::default());
        self.edge_arrowheads.push(EdgeArrowheads::default());
        self.edge_arrowhead_declarations
            .push(EdgeArrowheadDeclarations::default());
        self.edge_labels.push(None);
        self.edge_label_percentage_presence.push(None);
        self.edge_font_sizes.push(16);
        self.edge_label_bold.push(false);
        self.edge_arrowhead_labels
            .push(EdgeArrowheadLabels::default());
        self.edge_styles.push(EdgeStyle::default());
        self.edge_links.push(None);
        self.edge_tooltips.push(None);
        self.edge_icons.push(None);
        self.edge_icon_positions.push(None);
        self.edge_icon_border_radii.push(0.0);
        self.edge_is_curves.push(false);
        self.edge_table_columns.push(EdgeTableColumns::default());
        self.edge_routes.push(Vec::new());
        self.d2_input_routes.push(None);
        self.d2_edge_validation.push(None);
        self.edge_min_widths.push(None);
        self.edge_min_heights.push(None);
        id
    }

    /// Retains serialized minimum edge dimensions when an upstream graph has
    /// already measured its labels and arrowheads.
    pub fn set_edge_min_dimensions(&mut self, id: EdgeId, width: Option<f64>, height: Option<f64>) {
        if self.edge_min_widths.len() <= id.0 as usize {
            self.edge_min_widths.resize(self.edges.len(), None);
        }
        if self.edge_min_heights.len() <= id.0 as usize {
            self.edge_min_heights.resize(self.edges.len(), None);
        }
        self.edge_min_widths[id.0 as usize] = width;
        self.edge_min_heights[id.0 as usize] = height;
    }

    /// Stores an existing source-to-target route for standalone routing.
    ///
    /// This method does nothing for an unknown edge ID.
    pub fn set_edge_route(&mut self, id: EdgeId, route: Vec<Point>) {
        self.tala_decoded_routes.remove(&id);
        if let Some(value) = self.edge_routes.get_mut(id.0 as usize) {
            *value = route;
        }
        // A caller replacing a route supplies new validation input too.
        if let Some(value) = self.d2_input_routes.get_mut(id.0 as usize) {
            *value = None;
        }
        if let Some(Some(value)) = self.d2_edge_validation.get_mut(id.0 as usize) {
            value.nil_route_points.clear();
        }
    }

    /// Retains original D2 route order independently of working geometry.
    pub fn set_d2_input_route(&mut self, id: EdgeId, route: Vec<Point>) {
        if id.0 as usize >= self.edges.len() {
            return;
        }
        self.d2_input_routes.resize(self.edges.len(), None);
        self.d2_input_routes[id.0 as usize] = Some(route);
        if let Some(Some(value)) = self.d2_edge_validation.get_mut(id.0 as usize) {
            value.nil_route_points.clear();
        }
    }

    /// Returns original D2 validation points, or the working route for callers
    /// that have not discarded or canonicalized their input geometry.
    /// Nil point slots are identified by `D2EdgeValidation::nil_route_points`
    /// and must be rejected before these coordinates are used as geometry.
    pub fn d2_input_route(&self, id: EdgeId) -> &[Point] {
        self.d2_input_routes
            .get(id.0 as usize)
            .and_then(Option::as_deref)
            .unwrap_or_else(|| self.edge_route(id))
    }

    /// Retains original D2 edge attributes in caller endpoint order.
    pub fn set_d2_edge_validation(&mut self, id: EdgeId, value: D2EdgeValidation) {
        if id.0 as usize >= self.edges.len() {
            return;
        }
        self.d2_edge_validation.resize(self.edges.len(), None);
        self.d2_edge_validation[id.0 as usize] = Some(value);
    }

    /// Original D2 attributes, when supplied by the adapter or compiler.
    pub fn d2_edge_validation(&self, id: EdgeId) -> Option<&D2EdgeValidation> {
        self.d2_edge_validation
            .get(id.0 as usize)
            .and_then(Option::as_ref)
    }

    /// Returns an edge's stored route, or an empty slice when absent.
    pub fn edge_route(&self, id: EdgeId) -> &[Point] {
        self.edge_routes
            .get(id.0 as usize)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Sets SQL-table endpoint columns for an edge.
    ///
    /// The zero-based values select rendered table rows at the source and
    /// target ends. This method does nothing for an unknown edge ID.
    pub fn set_edge_table_columns(&mut self, id: EdgeId, columns: EdgeTableColumns) {
        if let Some(value) = self.edge_table_columns.get_mut(id.0 as usize) {
            *value = columns;
        }
    }

    /// Returns SQL-table endpoint columns for an edge.
    pub fn edge_table_columns(&self, id: EdgeId) -> EdgeTableColumns {
        self.edge_table_columns
            .get(id.0 as usize)
            .copied()
            .unwrap_or_default()
    }

    /// Sets arrow presence at both ends of an edge.
    ///
    /// This method does nothing for an unknown edge ID.
    pub fn set_edge_arrows(&mut self, id: EdgeId, arrows: EdgeArrows) {
        if let Some(value) = self.edge_arrows.get_mut(id.0 as usize) {
            *value = arrows;
        }
    }

    /// Returns arrow presence at both ends of an edge.
    pub fn edge_arrows(&self, id: EdgeId) -> EdgeArrows {
        self.edge_arrows
            .get(id.0 as usize)
            .copied()
            .unwrap_or_default()
    }

    /// Sets explicit arrowhead shapes for an edge.
    ///
    /// This method does nothing for an unknown edge ID.
    pub fn set_edge_arrowheads(&mut self, id: EdgeId, arrowheads: EdgeArrowheads) {
        if let Some(value) = self.edge_arrowheads.get_mut(id.0 as usize) {
            *value = arrowheads;
        }
    }

    /// Returns explicit arrowhead shapes for an edge.
    pub fn edge_arrowheads(&self, id: EdgeId) -> EdgeArrowheads {
        self.edge_arrowheads
            .get(id.0 as usize)
            .cloned()
            .unwrap_or_default()
    }

    /// Sets original compiled D2 arrowhead shapes without changing resolved identities.
    ///
    /// Unknown edge IDs are ignored. The vector may be absent in legacy JSON,
    /// so materialize its missing entries before setting a valid edge.
    pub fn set_edge_arrowhead_declarations(
        &mut self,
        id: EdgeId,
        declarations: EdgeArrowheadDeclarations,
    ) {
        if id.0 as usize >= self.edges.len() {
            return;
        }
        self.edge_arrowhead_declarations
            .resize(self.edges.len(), EdgeArrowheadDeclarations::default());
        self.edge_arrowhead_declarations[id.0 as usize] = declarations;
    }

    /// Returns original compiled D2 shapes, defaulting absent metadata to no records.
    pub fn edge_arrowhead_declarations(&self, id: EdgeId) -> EdgeArrowheadDeclarations {
        self.edge_arrowhead_declarations
            .get(id.0 as usize)
            .cloned()
            .unwrap_or_default()
    }

    /// Sets or removes an edge label.
    ///
    /// This method does nothing for an unknown edge ID.
    pub fn set_edge_label(&mut self, id: EdgeId, label: Option<EdgeLabel>) {
        if let Some(value) = self.edge_labels.get_mut(id.0 as usize) {
            *value = label;
        }
    }

    /// Sets adapter presence without changing the numeric label percentage.
    pub fn set_edge_label_percentage_present(&mut self, id: EdgeId, present: bool) {
        if id.0 as usize >= self.edges.len() {
            return;
        }
        self.edge_label_percentage_presence
            .resize(self.edges.len(), None);
        self.edge_label_percentage_presence[id.0 as usize] = Some(present);
    }

    /// Whether a numeric percentage is explicitly present in the source.
    pub fn edge_label_percentage_present(&self, id: EdgeId) -> bool {
        self.edge_label(id).is_some()
            && self
                .edge_label_percentage_presence
                .get(id.0 as usize)
                .copied()
                .flatten()
                .unwrap_or(true)
    }

    /// Returns an edge label.
    pub fn edge_label(&self, id: EdgeId) -> Option<&EdgeLabel> {
        self.edge_labels.get(id.0 as usize).and_then(Option::as_ref)
    }

    /// Returns a mutable edge label.
    pub fn edge_label_mut(&mut self, id: EdgeId) -> Option<&mut EdgeLabel> {
        self.edge_labels
            .get_mut(id.0 as usize)
            .and_then(Option::as_mut)
    }

    /// Sets the effective edge-label font size used by native rendering.
    pub fn set_edge_font_size(&mut self, id: EdgeId, size: u32) {
        if self.edge_font_sizes.len() <= id.0 as usize {
            self.edge_font_sizes.resize(self.edges.len(), 16);
        }
        self.edge_font_sizes[id.0 as usize] = size.max(1);
    }

    /// Returns the effective edge-label font size, defaulting to D2's 16px
    /// medium size for graphs reconstructed without this metadata.
    pub fn edge_font_size(&self, id: EdgeId) -> u32 {
        self.edge_font_sizes
            .get(id.0 as usize)
            .copied()
            .unwrap_or(16)
    }

    /// Sets the explicit bold state used to render an edge label.
    pub fn set_edge_label_bold(&mut self, id: EdgeId, bold: bool) {
        if self.edge_label_bold.len() <= id.0 as usize {
            self.edge_label_bold.resize(self.edges.len(), false);
        }
        self.edge_label_bold[id.0 as usize] = bold;
    }

    /// Returns whether an edge label has explicit bold styling.
    pub fn edge_label_bold(&self, id: EdgeId) -> bool {
        self.edge_label_bold
            .get(id.0 as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Sets arrowhead labels for an edge.
    ///
    /// This method does nothing for an unknown edge ID.
    pub fn set_edge_arrowhead_labels(&mut self, id: EdgeId, labels: EdgeArrowheadLabels) {
        if let Some(value) = self.edge_arrowhead_labels.get_mut(id.0 as usize) {
            *value = labels;
        }
    }

    /// Returns arrowhead labels for an edge.
    pub fn edge_arrowhead_labels(&self, id: EdgeId) -> EdgeArrowheadLabels {
        self.edge_arrowhead_labels
            .get(id.0 as usize)
            .cloned()
            .unwrap_or_default()
    }

    /// Sets explicit rendered style fields for an edge.
    ///
    /// This method does nothing for an unknown edge ID.
    pub fn set_edge_style(&mut self, id: EdgeId, style: EdgeStyle) {
        if let Some(value) = self.edge_styles.get_mut(id.0 as usize) {
            *value = style;
        }
    }

    /// Returns explicit rendered style fields for an edge.
    pub fn edge_style(&self, id: EdgeId) -> EdgeStyle {
        self.edge_styles
            .get(id.0 as usize)
            .cloned()
            .unwrap_or_default()
    }

    /// Sets an edge hyperlink. This method does nothing for an unknown edge.
    pub fn set_edge_link(&mut self, id: EdgeId, link: Option<String>) {
        if let Some(value) = self.edge_links.get_mut(id.0 as usize) {
            *value = link;
        }
    }

    /// Returns an edge hyperlink, if one was supplied.
    pub fn edge_link(&self, id: EdgeId) -> Option<&str> {
        self.edge_links
            .get(id.0 as usize)
            .and_then(Option::as_deref)
    }

    /// Sets an edge tooltip. This method does nothing for an unknown edge.
    pub fn set_edge_tooltip(&mut self, id: EdgeId, tooltip: Option<String>) {
        if let Some(value) = self.edge_tooltips.get_mut(id.0 as usize) {
            *value = tooltip;
        }
    }

    /// Stores an edge icon URL and its target placement metadata.
    pub fn set_edge_icon(
        &mut self,
        id: EdgeId,
        icon: Option<String>,
        position: Option<LabelPosition>,
        border_radius: f64,
    ) {
        let index = id.0 as usize;
        if self.edge_icons.len() <= index {
            self.edge_icons.resize(self.edges.len(), None);
        }
        if self.edge_icon_positions.len() <= index {
            self.edge_icon_positions.resize(self.edges.len(), None);
        }
        if self.edge_icon_border_radii.len() <= index {
            self.edge_icon_border_radii.resize(self.edges.len(), 0.0);
        }
        self.edge_icons[index] = icon;
        self.edge_icon_positions[index] = position;
        self.edge_icon_border_radii[index] = if border_radius.is_finite() {
            border_radius.max(0.0)
        } else {
            0.0
        };
    }

    /// Returns an edge icon URL, when one was supplied by the source target.
    pub fn edge_icon(&self, id: EdgeId) -> Option<&str> {
        self.edge_icons
            .get(id.0 as usize)
            .and_then(Option::as_deref)
            .filter(|icon| !icon.is_empty())
    }

    /// Returns the requested edge-icon label position.
    pub fn edge_icon_position(&self, id: EdgeId) -> Option<LabelPosition> {
        self.edge_icon_positions
            .get(id.0 as usize)
            .copied()
            .flatten()
    }

    /// Returns the requested edge-icon clipping radius.
    pub fn edge_icon_border_radius(&self, id: EdgeId) -> f64 {
        self.edge_icon_border_radii
            .get(id.0 as usize)
            .copied()
            .unwrap_or(0.0)
    }

    /// Returns an edge tooltip, if one was supplied.
    pub fn edge_tooltip(&self, id: EdgeId) -> Option<&str> {
        self.edge_tooltips
            .get(id.0 as usize)
            .and_then(Option::as_deref)
    }

    /// Sets whether an edge route should be rendered as a cubic curve.
    pub fn set_edge_is_curve(&mut self, id: EdgeId, is_curve: bool) {
        if let Some(value) = self.edge_is_curves.get_mut(id.0 as usize) {
            *value = is_curve;
        }
    }

    /// Returns whether an edge route is an explicit cubic curve.
    pub fn edge_is_curve(&self, id: EdgeId) -> bool {
        self.edge_is_curves
            .get(id.0 as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Iterates over nodes in stable ID order.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = (NodeId, &Node)> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (NodeId(i as u32), node))
    }

    /// Appends compiler provenance before the graph enters layout.
    pub fn source_metadata_mut(&mut self) -> &mut SourceGraphMetadata {
        std::sync::Arc::make_mut(self.source_metadata.get_or_insert_default())
    }

    /// Iterates over edges in stable ID order.
    pub fn edges(&self) -> impl ExactSizeIterator<Item = (EdgeId, &Edge)> {
        self.edges
            .iter()
            .enumerate()
            .map(|(i, edge)| (EdgeId(i as u32), edge))
    }

    /// Returns an edge by ID.
    pub fn edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.get(id.0 as usize)
    }

    /// Returns a node by ID.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0 as usize)
    }

    /// Resolves a connection endpoint, including the synthetic source root.
    /// Physical layout nodes remain available through `node`/`nodes` only.
    pub fn endpoint_node(&self, id: NodeId) -> Option<&Node> {
        if id == NodeId::ROOT {
            self.root_object.as_deref()
        } else {
            self.node(id)
        }
    }

    /// Returns a mutable node by ID.
    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.0 as usize)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Candidate seeds evaluated by a layout call.
pub struct LayoutOptions {
    /// Ordered candidate seeds. At least one seed is required.
    pub seeds: Vec<i64>,
    /// Maximum number of seed attempts that may run concurrently.
    ///
    /// Zero selects the D2/TALA default (up to four workers, limited by the
    /// number of normalized seeds). Values above sixteen or below one are
    /// rejected when nonzero.
    #[serde(default)]
    pub max_concurrency: usize,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            seeds: vec![1, 2, 3],
            max_concurrency: 0,
        }
    }
}

impl LayoutOptions {
    /// Maximum number of distinct seeds accepted by TALA.
    pub const MAX_SEEDS: usize = 16;
    /// Maximum number of raw seed entries accepted before deduplication.
    pub const MAX_SEED_ENTRIES: usize = Self::MAX_SEEDS * 4;
    /// Default worker bound used when [`Self::max_concurrency`] is zero.
    pub const DEFAULT_MAX_CONCURRENCY: usize = 4;

    /// Deduplicates and validates the configured seed list in source order.
    pub fn normalized_seeds(&self) -> Result<Vec<i64>, LayoutError> {
        if self.seeds.is_empty() {
            return Err(LayoutError::NoSeeds);
        }
        if self.seeds.len() > Self::MAX_SEED_ENTRIES {
            return Err(LayoutError::TooManySeedEntries {
                count: self.seeds.len(),
                max: Self::MAX_SEED_ENTRIES,
            });
        }
        let mut normalized = Vec::with_capacity(self.seeds.len().min(Self::MAX_SEEDS));
        for seed in self.seeds.iter().copied() {
            if normalized.contains(&seed) {
                continue;
            }
            normalized.push(seed);
            if normalized.len() > Self::MAX_SEEDS {
                return Err(LayoutError::TooManyUniqueSeeds {
                    max: Self::MAX_SEEDS,
                });
            }
        }
        Ok(normalized)
    }

    /// Resolves the bounded worker count for a normalized seed list.
    pub fn effective_max_concurrency(&self, seed_count: usize) -> Result<usize, LayoutError> {
        let requested = self.max_concurrency;
        if requested > Self::MAX_SEEDS {
            return Err(LayoutError::InvalidMaxConcurrency {
                value: requested,
                max: Self::MAX_SEEDS,
            });
        }
        Ok(if requested == 0 {
            Self::DEFAULT_MAX_CONCURRENCY
                .min(available_parallelism())
                .min(seed_count.max(1))
        } else {
            requested.min(seed_count.max(1))
        })
    }
}

/// `available_parallelism` reads the affinity mask and cgroup quota files on
/// every call, and each layout call (one per TALA grid cell) asks for it
/// before spawning its seed workers. Read it once per process. The worker
/// count only bounds concurrency: the seed reduction is index-ordered.
fn available_parallelism() -> usize {
    static AVAILABLE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *AVAILABLE
        .get_or_init(|| std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get))
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
/// Lexicographically ordered layout quality metrics; lower is better.
pub struct Score {
    /// Count of invalid geometry conditions.
    pub invalidities: u64,
    /// Count of route collisions.
    pub route_collisions: u64,
    /// Count of edge crossings.
    pub crossings: u64,
    /// Count of label collisions.
    pub label_collisions: u64,
    /// Count of route bends.
    pub bends: u64,
    /// Total route length multiplied by 1,000.
    pub route_length_milli: u64,
    /// Bounding area multiplied by 1,000.
    pub area_milli: u64,
    /// Count of node pairs that miss preferred alignment.
    pub unaligned_pairs: u64,
}

impl Ord for Score {
    fn cmp(&self, other: &Self) -> Ordering {
        (
            self.invalidities,
            self.route_collisions,
            self.crossings,
            self.label_collisions,
            self.bends,
            self.route_length_milli,
            self.area_milli,
            self.unaligned_pairs,
        )
            .cmp(&(
                other.invalidities,
                other.route_collisions,
                other.crossings,
                other.label_collisions,
                other.bends,
                other.route_length_milli,
                other.area_milli,
                other.unaligned_pairs,
            ))
    }
}

impl PartialOrd for Score {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Diagnostic record for one completed seed candidate.
pub struct CandidateReport {
    /// Seed used for the candidate.
    pub seed: i64,
    /// Candidate pass number.
    pub pass: usize,
    /// Geometry score published for diagnostics.
    pub score: Score,
    /// The quality penalty compared by D2 v0.9.0's RaceSeeds selector. The
    /// selector compares exact penalty first, then unrounded graph area from
    /// `race_score_components`; standalone route results omit both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub race_score: Option<f64>,
    /// Optional component breakdown for the TALA-compatible RaceSeeds scalar.
    /// This is report-only and does not alter the normal plugin protocol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub race_score_components: Option<RaceScoreComponents>,
    /// Diagnostic-only final internal edge order used by RaceSeeds scoring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub race_seed_edge_order: Option<Vec<EdgeId>>,
    /// Whether the candidate passed geometry validation.
    pub accepted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
/// Diagnostic components of seed selection and its recovered legacy scorer.
pub struct RaceScoreComponents {
    /// Route-turn contribution.
    pub route_turns: f64,
    /// Diagonal-segment contribution.
    pub diagonal_segments: f64,
    /// Crossing contribution.
    pub crossings: f64,
    /// Raw label score.
    pub label_score: f64,
    /// Weighted label penalty.
    pub label_penalty: f64,
    /// Unrounded graph area used as the selector's strict second key.
    #[serde(default)]
    pub selection_area: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Selection diagnostics for a layout or standalone-routing call.
pub struct LayoutReport {
    /// Seed of the published candidate.
    pub selected_seed: i64,
    /// Pass of the published candidate.
    pub selected_pass: usize,
    /// Score of the published geometry.
    pub score: Score,
    /// Number of completed candidates considered.
    pub improvement_passes: usize,
    /// Completed candidates in requested seed order.
    pub candidates: Vec<CandidateReport>,
    /// Non-fatal conditions encountered during selection.
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
/// A sequence actor's synthetic lifeline, retained across nested layout passes.
pub struct SequenceLifeline {
    /// Actor owning the lifeline.
    pub actor: NodeId,
    /// Stable synthetic destination identity (it has no obstacle box).
    pub target_id: String,
    /// Untrimmed route produced by sequence layout.
    pub route: Vec<Point>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
/// Retained synthetic source-root state. It never becomes an obstacle box.
pub struct SyntheticRootState {
    /// Nullable source position (ordinary LayoutNested roots remain unset).
    pub top_left: Option<Point>,
    /// Source root dimensions, independent of canvas/target bounds.
    pub size: Size,
    /// Position committed by a specialized layout, if any.
    pub label_position: Option<LabelPosition>,
    /// Icon position committed by a specialized layout, if any.
    pub icon_position: Option<LabelPosition>,
}

/// Mutation of the containing Go Graph's edge slice during layout.
/// Child reinjection does not transfer the child's empty storage state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceEdgeStorage {
    /// The backend preserves the compiler's initially nil empty slice.
    #[default]
    Unchanged,
    /// ExtractSubgraph allocated the containing graph's remaining-edge slice.
    Allocated,
    /// A TALA patch replaced the slice with its initially nil final-edge list.
    Replaced,
}

impl SourceEdgeStorage {
    /// Record extraction that occurred before the current backend commit.
    pub fn record_prior_extraction(&mut self) {
        if *self != Self::Replaced {
            *self = Self::Allocated;
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Geometry and diagnostics produced by the layout engine.
pub struct LayoutResult {
    /// Input edges permanently consumed by layout (TALA step sequences).
    /// IDs remain in the input arena; nested projections remap this inventory
    /// alongside routes. Export omits only these explicit removals.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub removed_edges: BTreeSet<EdgeId>,
    /// Source edge-slice lifetime, distinct from the number of routed edges.
    #[serde(skip)]
    pub source_edge_storage: SourceEdgeStorage,
    /// Source root state, kept out of legacy native result JSON.
    #[serde(skip)]
    pub synthetic_root: Option<SyntheticRootState>,
    /// Source route kinds, kept separately from point count and wire metadata.
    #[doc(hidden)]
    #[serde(skip)]
    pub route_curves: BTreeMap<EdgeId, bool>,
    /// Synthetic lifelines in sequence layout/restoration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequence_lifelines: Vec<SequenceLifeline>,
    /// Final node rectangles keyed by input node ID.
    pub boxes: BTreeMap<NodeId, Rect>,
    /// Final source-to-target edge routes keyed by input edge ID.
    pub routes: BTreeMap<EdgeId, Vec<Point>>,
    /// Final edge-label placements keyed by input edge ID.
    pub edge_labels: BTreeMap<EdgeId, EdgeLabel>,
    /// Labels whose percentage was explicitly committed by an engine patch.
    /// Local grid/sequence position changes do not create a percentage value.
    #[doc(hidden)]
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub edge_label_percentage_commits: BTreeSet<EdgeId>,
    /// Final node-label and icon state keyed by input node ID.
    pub node_labels: BTreeMap<NodeId, NodeLabelState>,
    /// Candidate-selection diagnostics.
    pub report: LayoutReport,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
/// Final rendered label and icon state for a node.
pub struct NodeLabelState {
    /// TALA's layout commit materializes the object's font-size scalar.
    /// Initial/route-only label states do not write source attributes.
    #[serde(skip)]
    pub font_size_committed: bool,
    /// Final label dimensions.
    pub size: Option<Size>,
    /// Final font size.
    pub font_size: Option<u32>,
    /// Final label position.
    pub position: LabelPosition,
    #[serde(default)]
    /// Final icon position.
    pub icon_position: Option<LabelPosition>,
}

fn go_position_coordinate(value: f64) -> String {
    if value == f64::INFINITY {
        "+Inf".into()
    } else if value == f64::NEG_INFINITY {
        "-Inf".into()
    } else if value.is_nan() {
        "NaN".into()
    } else {
        format!("{value:.6}")
    }
}

#[derive(Clone, Debug, Error)]
/// Error returned when input validation, layout, or routing cannot complete.
pub enum LayoutError {
    /// An original D2 operation dereferences a missing pointer. Rust reports
    /// the recovered fault without trapping its WASM instance. Source context
    /// identifies the mapped operation; it is not a fabricated Go backtrace.
    #[error(
        "panic recovered: runtime error: invalid memory address or nil pointer dereference\nnative source context: {source_site}"
    )]
    D2NilPointerFault {
        /// Original operation at the failed pointer access.
        source_site: &'static str,
    },
    /// Initialization exhausted the source's bounded unoccupied-point search.
    #[error("no unoccupied points within d=100 of p=&{{{} {}}}", layout_dimension_text(point.x), layout_dimension_text(point.y))]
    NoUnoccupiedPoint {
        /// Median point in sizeless coordinates.
        point: Point,
    },
    /// An original D2 topology or identity failed source adapter validation.
    #[error("{0}")]
    D2GraphValidation(String),
    /// Direct native engine validation failed after adapter reconstruction.
    #[error("{0}")]
    EngineValidation(String),
    /// An original D2 label or style declaration failed ordered validation.
    #[error("{0}")]
    D2AttributeValidation(String),
    /// The original edge label percentage is outside its supported domain.
    #[error("D2 edge {} has invalid label percentage {}: must be finite and between 0 and 1", edge.0, layout_dimension_text(*value))]
    D2LabelPercentage {
        /// Original edge inventory slot.
        edge: EdgeId,
        /// Original percentage, before source-arrow reversal.
        value: f64,
    },
    /// Stored routes exceed D2's aggregate point limit.
    #[error("D2 route point count exceeds limit {limit}")]
    D2RoutePointLimit {
        /// Maximum total points across all stored edge routes.
        limit: usize,
    },
    /// A stored route contains exactly one point.
    #[error("D2 edge {} route requires either zero or at least two points", edge.0)]
    D2SingleRoutePoint {
        /// Edge's slot in the caller's inventory.
        edge: EdgeId,
    },
    /// A stored route point has an unsupported coordinate.
    #[error("D2 edge {} route point {index}: route {axis} is outside the finite supported range: {}", edge.0, layout_dimension_text(*value))]
    D2RouteCoordinate {
        /// Edge's slot in the caller's inventory.
        edge: EdgeId,
        /// Point's zero-based index.
        index: usize,
        /// Coordinate axis.
        axis: &'static str,
        /// Supplied coordinate.
        value: f64,
    },
    /// Two immediately adjacent stored route points coincide.
    #[error("D2 edge {} has a degenerate route segment ending at index {index}", edge.0)]
    D2DegenerateRoute {
        /// Edge's slot in the caller's inventory.
        edge: EdgeId,
        /// Index of the segment's second point.
        index: usize,
    },
    /// A partial reroute needs an existing route on every unselected edge.
    #[error("unselected edge {entity_id} has no complete route")]
    UnselectedRouteMissing {
        /// Translated TALA identity, rather than the input edge slot.
        entity_id: i64,
    },
    /// Imported route repeats an identical visibility-graph node.
    #[error("found duplicate OVGNode (&{{X:{} Y:{}}}) at index {index} in route {edge}", layout_dimension_text(point.x), layout_dimension_text(point.y))]
    DuplicateRouteOvgNode {
        /// Coordinate of the repeated node.
        point: Point,
        /// Source duplicate-scan index (after dropping a loop's first center).
        index: usize,
        /// Source-compatible edge debug identity.
        edge: String,
    },
    /// A source invariant check rejected an internal layout state.
    #[error("layout invariant violated: {0}")]
    InvariantViolation(String),
    /// The selected TALA route flavor exhausted its independent search budget.
    #[error(
        "TALA route-search work limit exceeded: TALA EdgeRouting flavor {flavor} work exceeds limit {limit}"
    )]
    RouteSearchWorkLimit {
        /// Declared route-generation flavor.
        flavor: &'static str,
        /// Independent per-flavor work limit.
        limit: u64,
    },
    /// Source seed data rejected when the TALA core is actually invoked.
    #[error("{0}")]
    InvalidSeedData(String),
    /// The graph contains more nodes or edges than its 32-bit typed IDs can
    /// represent.
    #[error("graph contains more entries than typed IDs can address")]
    IdOverflow,
    /// A node has a non-finite, zero, or negative rendered dimension.
    #[error("node {node:?} has invalid size {width}x{height}")]
    InvalidSize {
        /// Node whose size failed validation.
        node: NodeId,
        /// Supplied width.
        width: f64,
        /// Supplied height.
        height: f64,
    },
    /// Standalone D2 routing requires an existing position for every object.
    #[error("D2 object {} has no top-left position", go_quote::quote(object))]
    D2MissingPosition {
        /// Original local D2 object identity.
        object: Vec<u8>,
    },
    /// An existing D2 object box has an unsupported dimension.
    #[error("D2 object {}: {dimension} is outside the supported range: {}", go_quote::quote(object), layout_dimension_text(*value))]
    D2ObjectDimension {
        /// Original local D2 object identity.
        object: Vec<u8>,
        /// Width or height.
        dimension: &'static str,
        /// Supplied dimension.
        value: f64,
    },
    /// An existing D2 object box has an unsupported coordinate or extent.
    #[error("D2 object {}: {coordinate} is outside the finite supported range: {}", go_quote::quote(object), layout_dimension_text(*value))]
    D2ObjectCoordinate {
        /// Original local D2 object identity.
        object: Vec<u8>,
        /// Corner and axis checked by the source adapter.
        coordinate: &'static str,
        /// Supplied or computed coordinate.
        value: f64,
    },
    /// Original D2 rejects infinite final coordinates after nested layout.
    #[error("object {} has invalid position with infinity value: x={}, y={}", go_quote::quote(object.as_bytes()), go_position_coordinate(*x), go_position_coordinate(*y))]
    InvalidPosition {
        /// Absolute D2 object identity.
        object: String,
        /// Final horizontal coordinate.
        x: f64,
        /// Final vertical coordinate.
        y: f64,
    },
    /// A node's parent ID does not exist in the graph.
    #[error("node {node:?} has a missing parent {parent:?}")]
    MissingParent {
        /// Child containing the invalid reference.
        node: NodeId,
        /// Missing parent ID.
        parent: NodeId,
    },
    /// Following parent references from a node eventually returns to that
    /// node instead of reaching the root scope.
    #[error("node {node:?} participates in a parent cycle")]
    ParentCycle {
        /// A node participating in the cycle.
        node: NodeId,
    },
    /// An edge's source or target ID does not exist in the graph.
    #[error("edge {edge:?} references a missing node")]
    MissingEndpoint {
        /// Edge containing the invalid endpoint.
        edge: EdgeId,
    },
    /// A standalone D2 routing selection exceeds the source adapter limit.
    #[error("requested D2 edge count exceeds limit {limit}")]
    D2RequestedEdgeLimit {
        /// Maximum accepted selection length.
        limit: usize,
    },
    /// A standalone D2 routing selection repeats an earlier edge.
    #[error("requested D2 edge {index} is repeated")]
    D2RequestedEdgeRepeated {
        /// Position of the repeated reference in the selection.
        index: usize,
    },
    /// A D2 graph exceeds the source adapter's edge inventory limit.
    #[error("D2 edge count exceeds limit {limit}")]
    D2EdgeLimit {
        /// Maximum accepted graph edge count.
        limit: usize,
    },
    /// A routing selection contains more references than the graph has edges.
    #[error("requested D2 edge count {requested} exceeds graph edge count {available}")]
    D2RequestedEdgeCount {
        /// Number of requested references.
        requested: usize,
        /// Number of edges in the graph.
        available: usize,
    },
    /// A routing selection names an edge outside the graph's ID inventory.
    #[error("requested D2 edge {index} is not in the graph")]
    D2RequestedEdgeMissing {
        /// Position of the missing edge reference in the selection.
        index: usize,
    },
    /// A compiled D2 arrowhead shape is outside TALA's adapter catalog.
    #[error("D2 edge {} has unsupported {endpoint} arrowhead {shape:?}", .edge.0)]
    UnsupportedD2Arrowhead {
        /// Edge containing the unsupported shape.
        edge: EdgeId,
        /// Original D2 endpoint: `source` or `destination`.
        endpoint: &'static str,
        /// Compiled shape before target arrowhead resolution.
        shape: String,
    },
    /// A compiled D2 scalar cannot be read by Go's platform-sized Atoi.
    #[error(
        "D2 object {} has invalid integer {attribute} attribute {}",
        go_quote::quote(object),
        go_quote::quote(value)
    )]
    D2InvalidIntegerAttribute {
        /// Original local object ID bytes.
        object: Vec<u8>,
        /// Width, height, top or left.
        attribute: &'static str,
        /// Original scalar bytes.
        value: Vec<u8>,
    },
    /// A compiled D2 integer exceeds the original adapter's attribute range.
    #[error(
        "D2 object {} {attribute} attribute must be between {min} and {max}",
        go_quote::quote(object)
    )]
    D2IntegerAttributeRange {
        /// Original local object ID bytes.
        object: Vec<u8>,
        /// Width, height, top or left.
        attribute: &'static str,
        /// Inclusive lower bound.
        min: isize,
        /// Inclusive upper bound.
        max: isize,
    },
    /// D2 locked positions require both authored axes.
    #[error(
        "D2 object {} must set top and left together for a locked position",
        go_quote::quote(object)
    )]
    D2UnpairedPosition {
        /// Original local object ID bytes.
        object: Vec<u8>,
    },
    /// A node's [`Node::near`] target does not exist in the graph.
    #[error("node {node:?} references a missing near object {near:?}")]
    MissingNear {
        /// Node containing the invalid reference.
        node: NodeId,
        /// Missing near-object ID.
        near: NodeId,
    },
    /// A D2 near key passed compiler validation but does not identify an
    /// actual object in the TALA input tree.
    #[error("D2 object {} has a near key outside the object tree", go_quote::quote(object.as_bytes()))]
    D2NearOutsideTree {
        /// Absolute object identity in the current input tree.
        object: String,
    },
    /// The D2 near key exceeds the original TALA adapter's depth bound.
    #[error("D2 object {} near key depth exceeds limit {max}", go_quote::quote(object.as_bytes()))]
    D2NearKeyDepth {
        /// Absolute object identity in the current input tree.
        object: String,
        /// Maximum supported component count.
        max: usize,
    },
    /// A D2 near key contains a nil or unboxable AST string component.
    #[error("D2 object {} has an invalid near key part at index {index}", go_quote::quote(object.as_bytes()))]
    D2InvalidNearKeyPart {
        /// Absolute object identity in the current input tree.
        object: String,
        /// First invalid component in source path order.
        index: usize,
    },
    /// A supplied D2 near key has no components.
    #[error("D2 object {} has an empty near key", go_quote::quote(object.as_bytes()))]
    D2EmptyNearKey {
        /// Absolute object identity in the current input tree.
        object: String,
    },
    /// Following near-object references forms a cycle.
    #[error("node {node:?} participates in a near-object cycle")]
    NearCycle {
        /// A node participating in the cycle.
        node: NodeId,
    },
    /// [`LayoutOptions::seeds`] was empty.
    #[error("tala requires at least one seed")]
    NoSeeds,
    /// More raw seed entries were supplied than TALA accepts.
    #[error("tala accepts at most {max} seed entries")]
    TooManySeedEntries {
        /// Number of entries supplied before deduplication.
        count: usize,
        /// Maximum accepted entries.
        max: usize,
    },
    /// More distinct seed values were supplied than TALA accepts.
    #[error("tala supports at most {max} unique seeds")]
    TooManyUniqueSeeds {
        /// Maximum accepted distinct values.
        max: usize,
    },
    /// The requested seed worker count is outside TALA's supported range.
    #[error(
        "tala max concurrency must be between 1 and {max}, or zero for the default (received {value})"
    )]
    InvalidMaxConcurrency {
        /// Supplied worker count.
        value: usize,
        /// Maximum accepted worker count.
        max: usize,
    },
    /// A named stage exceeded its allotted compatibility deadline.
    #[error("Timed out: {0}")]
    TimedOut(&'static str),
    /// The caller requested cooperative cancellation before layout could
    /// publish a complete candidate.
    #[error("context canceled")]
    Cancelled,
    /// The caller's context deadline expired before publication.
    #[error("context deadline exceeded")]
    DeadlineExceeded,
    /// An engine adapter adds context while preserving the underlying cause.
    #[error("{context}: {source}")]
    Context {
        /// Source adapter operation description.
        context: &'static str,
        /// Original error returned by the operation.
        source: Box<LayoutError>,
    },
    /// Standalone or full routing could not publish a route for every
    /// requested edge.
    #[error("could not route every requested edge")]
    EdgeRoutingFailed,
    /// elk-go could not resolve a configured algorithm for a populated scope.
    #[error("layout algorithm {} not found for {path}", go_quote::quote(algorithm.as_bytes()))]
    ElkAlgorithmNotFound {
        /// Original, case-sensitive option spelling.
        algorithm: String,
        /// ELK's node path (the synthetic root has an empty path).
        path: String,
    },
    /// Routing reached an internal state that violates a required invariant.
    #[error("Reached a bad state: {0}")]
    BadRouteState(String),
    /// d2sequence rejected the diagram; its message is reported verbatim.
    #[error("{0}")]
    SequenceDiagram(String),
    /// A guarded TALA placement transaction could not complete. Preserve the
    /// source diagnostic, including its work location or context cause.
    #[error("{0}")]
    PlacementTransaction(String),
    /// A completed TALA stage exceeds the source engine's graph dimension limit.
    #[error(
        "layout invariant violated: Dimensions w:{width_text}, h:{height_text} reached after stage {stage}",
        width_text = layout_dimension_text(*.width),
        height_text = layout_dimension_text(*.height)
    )]
    StageDimensions {
        /// Completed source stage name.
        stage: &'static str,
        /// Rounded graph bounding-box width.
        width: f64,
        /// Rounded graph bounding-box height.
        height: f64,
    },
    /// A seed attempt recovered an internal panic, as original runLocalSeed does.
    #[error("TALA seed layout failed due to an internal invariant")]
    SeedInvariant,
    /// Standalone TALA edge routing recovered an internal panic, as original
    /// RouteEdges does through `recoverAsError("edge routing", &err)`.
    #[error("TALA edge routing failed due to an internal invariant")]
    EdgeRoutingInvariant,
    /// Every configured TALA seed failed, in request order.
    #[error("all TALA seed attempts failed: {details}", details = layout_seed_failure_text(.attempts))]
    AllSeedAttemptsFailed {
        /// Actual seed and its underlying failure for each attempt.
        attempts: Vec<(i64, LayoutError)>,
    },
    /// The recovered CombineSubgraphs operation exceeded D2's aggregate
    /// engine work budget before a seed could publish a candidate.
    #[error(
        "all TALA seed attempts failed: seed {seed}: TALA {location} work exceeds limit {limit}"
    )]
    WorkLimit {
        /// Seed whose candidate hit the aggregate work budget.
        seed: i64,
        /// TALA operation that exhausted its budget.
        location: &'static str,
        /// Maximum accepted work units.
        limit: u64,
    },
}

// Go's default floating-point formatting uses shortest `%g`, including the
// exponent threshold and a signed, at least two-digit exponent.
fn layout_dimension_text(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-Inf"
        } else {
            "+Inf"
        }
        .into();
    }
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("scientific exponent");
    let exponent: i32 = exponent.parse().expect("formatted exponent");
    if !(-4..6).contains(&exponent) {
        format!("{mantissa}e{exponent:+03}")
    } else {
        value.to_string()
    }
}

fn layout_seed_failure_text(attempts: &[(i64, LayoutError)]) -> String {
    attempts
        .iter()
        .map(|(seed, error)| {
            // WorkLimit's existing public display includes the single-seed
            // adapter wrapper. A coordinator adds that wrapper exactly once.
            let detail = match error {
                LayoutError::WorkLimit {
                    location, limit, ..
                } => {
                    format!("TALA {location} work exceeds limit {limit}")
                }
                error => error.to_string(),
            };
            format!("seed {seed}: {detail}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod layout_options_tests {
    #[test]
    fn tala_dimension_error_matches_go_float_and_seed_formatting() {
        for (value, expected) in [
            (30_001.0, "30001"),
            (1_000_000.0, "1e+06"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (-0.0, "-0"),
            (f64::INFINITY, "+Inf"),
        ] {
            assert_eq!(super::layout_dimension_text(value), expected);
        }
        let error = super::LayoutError::AllSeedAttemptsFailed {
            attempts: vec![(
                17,
                super::LayoutError::WorkLimit {
                    seed: 17,
                    location: "CombineSubgraphs",
                    limit: 10,
                },
            )],
        };
        assert_eq!(
            error.to_string(),
            "all TALA seed attempts failed: seed 17: TALA CombineSubgraphs work exceeds limit 10"
        );
    }

    use super::{
        ExternalAlignment, ExternalLabel, ExternalSide, LabelPosition, LayoutError, LayoutOptions,
        Point, Rect, Size,
    };

    #[test]
    fn label_percentage_presence_distinguishes_missing_from_explicit_zero() {
        use super::{Edge, EdgeLabel, Graph, Node};
        let mut graph = Graph::default();
        let a = graph.add_node(Node::new("a", Size::default()));
        let edge = graph.add_edge(Edge {
            source: a,
            target: a,
        });
        graph.set_edge_label(
            edge,
            Some(EdgeLabel {
                text: "label".to_owned(),
                size: Size::default(),
                position: LabelPosition::InsideMiddleCenter,
                percentage: 0.0,
            }),
        );
        assert!(graph.edge_label_percentage_present(edge));
        graph.set_edge_label_percentage_present(edge, false);
        assert!(!graph.edge_label_percentage_present(edge));
        let label = graph.edge_label(edge).cloned();
        graph.set_edge_label(edge, label);
        assert!(!graph.edge_label_percentage_present(edge));
        assert_eq!(
            graph.clone().edge_label_percentage_presence,
            vec![Some(false)]
        );
        graph.set_edge_label_percentage_present(edge, true);
        assert!(graph.edge_label_percentage_present(edge));
    }

    #[test]
    fn automatic_external_label_tracks_selected_position() {
        let rect = Rect {
            origin: Point { x: 100.0, y: 200.0 },
            size: Size {
                width: 63.0,
                height: 66.0,
            },
        };
        let label = ExternalLabel {
            size: Size {
                width: 48.0,
                height: 21.0,
            },
            side: ExternalSide::Bottom,
            alignment: ExternalAlignment::Center,
            automatic: true,
            reserve_space: true,
        };
        assert_eq!(
            label
                .at_position(LabelPosition::OutsideTopCenter)
                .top_left(rect, 5.0),
            Point { x: 107.5, y: 174.0 }
        );
        assert_eq!(
            label
                .at_position(LabelPosition::OutsideRightBottom)
                .top_left(rect, 5.0),
            Point { x: 168.0, y: 240.0 }
        );
    }

    #[test]
    fn explicit_external_label_keeps_authored_position() {
        let label = ExternalLabel {
            size: Size {
                width: 48.0,
                height: 21.0,
            },
            side: ExternalSide::Bottom,
            alignment: ExternalAlignment::Center,
            automatic: false,
            reserve_space: true,
        };
        assert_eq!(
            label.at_position(LabelPosition::OutsideTopCenter).side,
            ExternalSide::Bottom
        );
    }

    #[test]
    fn normalize_seeds_preserves_first_occurrence_order() {
        let options = LayoutOptions {
            seeds: vec![3, 1, 3, 2, 1],
            max_concurrency: 0,
        };
        assert_eq!(options.normalized_seeds().unwrap(), vec![3, 1, 2]);
    }

    #[test]
    fn normalize_seeds_applies_entry_and_unique_limits() {
        let too_many_entries = LayoutOptions {
            seeds: vec![0; LayoutOptions::MAX_SEED_ENTRIES + 1],
            max_concurrency: 0,
        };
        assert!(matches!(
            too_many_entries.normalized_seeds(),
            Err(LayoutError::TooManySeedEntries { .. })
        ));

        let too_many_unique = LayoutOptions {
            seeds: (0..=LayoutOptions::MAX_SEEDS as i64).collect(),
            max_concurrency: 0,
        };
        assert!(matches!(
            too_many_unique.normalized_seeds(),
            Err(LayoutError::TooManyUniqueSeeds { .. })
        ));
    }

    #[test]
    fn effective_concurrency_matches_tala_defaults_and_clamps_to_seeds() {
        let defaulted = LayoutOptions::default();
        assert!((1..=3).contains(&defaulted.effective_max_concurrency(3).unwrap()));

        let requested = LayoutOptions {
            seeds: vec![1, 2],
            max_concurrency: 16,
        };
        assert_eq!(requested.effective_max_concurrency(2).unwrap(), 2);

        let invalid = LayoutOptions {
            seeds: vec![1],
            max_concurrency: LayoutOptions::MAX_SEEDS + 1,
        };
        assert!(matches!(
            invalid.effective_max_concurrency(1),
            Err(LayoutError::InvalidMaxConcurrency { .. })
        ));
    }

    #[test]
    fn cancellation_token_is_shared_by_clones() {
        let token = super::CancellationToken::new();
        let clone = token.clone();
        assert!(!token.is_cancelled());
        clone.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn context_cause_preserves_first_parent_or_child_cancellation() {
        for deadline_first in [false, true] {
            let parent = super::CancellationToken::new();
            let child = parent.child();
            let grandchild = child.child();
            if deadline_first {
                parent.expire();
                parent.cancel();
            } else {
                parent.cancel();
                parent.expire();
            }
            let expected = if deadline_first {
                "context deadline exceeded"
            } else {
                "context canceled"
            };
            for token in [&parent, &child, &grandchild, &parent.child()] {
                assert_eq!(token.error().unwrap().to_string(), expected);
            }
        }
        let parent = super::CancellationToken::new();
        let expired = parent.child();
        let sibling = parent.child();
        expired.expire();
        assert!(parent.error().is_none());
        assert!(sibling.error().is_none());
        parent.cancel();
        assert_eq!(
            expired.error().unwrap().to_string(),
            "context deadline exceeded"
        );
        assert_eq!(sibling.error().unwrap().to_string(), "context canceled");
    }

    #[test]
    fn child_registration_cannot_miss_concurrent_parent_cancellation() {
        for _ in 0..64 {
            let parent = super::CancellationToken::new();
            std::thread::scope(|scope| {
                let worker = scope.spawn(|| parent.expire());
                let child = parent.child();
                worker.join().unwrap();
                assert!(matches!(child.error(), Some(LayoutError::DeadlineExceeded)));
            });
        }
    }

    #[test]
    fn repeated_cancellation_waits_for_descendant_propagation() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};
        for deadline in [false, true] {
            let parent = super::CancellationToken::new();
            let child = parent.child();
            let grandchild = child.child();
            // Hold a descendant at a real propagation lock, after the root
            // publishes its cause. A repeated call must not pass that barrier.
            let held_child = child.state.children.lock().unwrap();
            std::thread::scope(|scope| {
                let first = scope.spawn(|| {
                    if deadline {
                        parent.expire();
                    } else {
                        parent.cancel();
                    }
                });
                let wait_until = Instant::now() + Duration::from_secs(5);
                while !parent.is_cancelled() && Instant::now() < wait_until {
                    std::thread::yield_now();
                }
                let published = parent.is_cancelled();
                let (started_tx, started_rx) = mpsc::channel();
                let (done_tx, done_rx) = mpsc::channel();
                let repeated = parent.clone();
                let second = scope.spawn(move || {
                    started_tx.send(()).unwrap();
                    repeated.cancel();
                    done_tx.send(()).unwrap();
                });
                started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                let early_completion = done_rx.recv_timeout(Duration::from_millis(100));
                // Release and join before asserting, including on a regression.
                drop(held_child);
                first.join().unwrap();
                second.join().unwrap();
                assert!(published);
                assert!(matches!(
                    early_completion,
                    Err(mpsc::RecvTimeoutError::Timeout)
                ));
                let expected = if deadline {
                    "context deadline exceeded"
                } else {
                    "context canceled"
                };
                for token in [&parent, &child, &grandchild] {
                    assert_eq!(token.error().unwrap().to_string(), expected);
                }
            });
        }
    }

    #[test]
    fn cancellation_of_deep_tree_is_stack_safe_and_complete() {
        let mut nodes = vec![super::CancellationToken::new()];
        for _ in 0..50_000 {
            nodes.push(nodes.last().unwrap().child());
        }
        nodes[0].expire();
        nodes[0].cancel();
        assert!(
            nodes
                .iter()
                .all(|node| matches!(node.error(), Some(LayoutError::DeadlineExceeded)))
        );
    }

    #[test]
    fn descendants_retain_dropped_intermediate_contexts() {
        let root = super::CancellationToken::new();
        let mut leaf = root.child();
        for _ in 0..50_000 {
            leaf = leaf.child();
        }
        root.expire();
        leaf.cancel();
        assert!(matches!(leaf.error(), Some(LayoutError::DeadlineExceeded)));
        // Dropping the last leaf also releases its complete ancestor chain.
        drop(root);
        drop(leaf);
    }

    #[test]
    fn dropping_an_active_deep_context_chain_is_stack_safe() {
        let mut leaf = super::CancellationToken::new();
        for _ in 0..50_000 {
            leaf = leaf.child();
        }
        assert!(leaf.error().is_none());
        drop(leaf);
    }
}

#[cfg(test)]
mod arrowhead_identity_tests {
    use super::resolve_d2_arrowhead_shape;

    #[test]
    fn filled_style_resolution_matches_d2target_arrowhead_identities() {
        for (shape, filled, expected) in [
            ("diamond", Some(true), "filled-diamond"),
            ("circle", Some(true), "filled-circle"),
            ("box", Some(true), "filled-box"),
            ("triangle", Some(false), "unfilled-triangle"),
            ("", Some(false), "unfilled-triangle"),
            ("", None, "triangle"),
            ("unknown", Some(false), "unfilled-triangle"),
            ("unknown", None, "triangle"),
            ("diamond", Some(false), "diamond"),
            ("arrow", Some(false), "arrow"),
        ] {
            assert_eq!(
                resolve_d2_arrowhead_shape(shape, filled),
                expected,
                "shape={shape:?}, filled={filled:?}"
            );
        }
    }
}

/// D2 v0.9.0 Edge.AbsID, including common scope and equivalent-edge index.
pub fn edge_abs_id(graph: &Graph, edge_id: EdgeId) -> Option<String> {
    let edge = graph.edges.get(edge_id.0 as usize)?;
    // d2graph.Edge.AbsID moves common ancestor segments outside the edge
    // expression. Derive segments from parent records: splitting on dots
    // would split an authored quoted identifier such as `"a.b"` too.
    fn segments(graph: &Graph, mut id: NodeId) -> Option<Vec<&str>> {
        let mut path = Vec::new();
        loop {
            let node = graph.endpoint_node(id)?;
            let local = if let Some(parent) = node.parent {
                node.external_id
                    .strip_prefix(&graph.node(parent)?.external_id)
                    .and_then(|suffix| suffix.strip_prefix('.'))
                    .unwrap_or(&node.external_id)
            } else {
                &node.external_id
            };
            path.push(local);
            let Some(parent) = node.parent else { break };
            id = parent;
        }
        path.reverse();
        Some(path)
    }
    let source = segments(graph, edge.source)?;
    let target = segments(graph, edge.target)?;
    let mut common = 0;
    while common + 1 < source.len()
        && common + 1 < target.len()
        && ir::equal_fold(source[common].as_bytes(), target[common].as_bytes())
    {
        common += 1;
    }
    let scope = if common == 0 {
        String::new()
    } else {
        format!("{}.", source[..common].join("."))
    };
    let arrows = graph.edge_arrows(edge_id);
    let arrow = match (arrows.source, arrows.target) {
        (true, true) => "<->",
        (true, false) => "<-",
        (false, true) => "->",
        (false, false) => "--",
    };
    let index = graph.edge_index(edge_id)?;
    Some(format!(
        "{scope}({} {arrow} {})[{index}]",
        source[common..].join("."),
        target[common..].join(".")
    ))
}

fn edge_indices_absent(indices: &[Option<usize>]) -> bool {
    indices.iter().all(Option::is_none)
}

#[cfg(test)]
mod public_c4_shape_wire_contract {
    #[test]
    fn canonical_public_shape_and_legacy_input_alias() {
        assert_eq!(
            serde_json::to_string(&super::ShapeKind::C4Person).unwrap(),
            "\"c4-person\""
        );
        for input in ["\"c4-person\"", "\"c4_person\""] {
            assert_eq!(
                serde_json::from_str::<super::ShapeKind>(input).unwrap(),
                super::ShapeKind::C4Person
            );
        }
    }
}

#[doc(hidden)]
pub mod raster_orientation;

#[doc(hidden)]
pub mod jpeg_idct;

#[doc(hidden)]
pub mod jpeg_entropy;

#[doc(hidden)]
pub mod jpeg_decode;

#[doc(hidden)]
pub mod jpeg_first;

impl LayoutResult {
    /// Explicit engine route kind, with the existing legacy-input fallback.
    pub fn route_is_curve(&self, graph: &Graph, id: EdgeId) -> bool {
        self.route_curves
            .get(&id)
            .copied()
            .unwrap_or_else(|| graph.edge_is_curve(id))
    }
}

#[cfg(test)]
mod synthetic_root_tests;
