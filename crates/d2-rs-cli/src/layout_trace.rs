// SPDX-License-Identifier: MPL-2.0
//! `--debug` layout records: d2lib/d2.go compileBoard's board walk (with its
//! layout-geometry reuse) and the log.Debug calls of d2layouts.LayoutNested
//! (d2layouts/d2layouts.go:231, 281, 287, 293, 372).
//!
//! The records depend only on the object tree, so they are replayed on a
//! structural model of each board's d2graph rather than on the engines' own
//! nested-layout implementations: ExtractSubgraph, extractSubgraphs,
//! InjectNested, d2near.Layout and SaveOrder move objects between graphs
//! exactly as Go does, and `Graph.PrintString` reads the moved order.

use super::cli_log;
use d2_rs_core::{CanvasPosition, Graph};
use std::collections::{HashMap, VecDeque};

/// One board of the compiled document, in d2lib compileBoard order.
struct Board {
    node: d2_rs_core::ir::NodeId,
    path: String,
    kind: BoardKind,
    children: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BoardKind {
    Layer,
    Scenario,
    Step,
}

/// Lowers one board to its measured graph (d2lib compileBoard's ApplyTheme
/// and SetDimensions). An error stops the replay, as it stops Go's compile.
pub(crate) type LowerBoard<'a> = dyn FnMut(d2_rs_core::ir::NodeId) -> Result<Graph, String> + 'a;

/// The core layout's input checks, applied where LayoutNested calls
/// coreLayout (d2layouts.go:293-297): a failing check ends Go's compile
/// right after that "default layout" record.
pub(crate) struct CoreChecks {
    /// The resolved engine (d2cli LayoutResolver matches case-insensitively).
    pub(crate) engine: String,
    /// `--elk-algorithm`: elk-go fails the layout for an unknown name.
    pub(crate) elk_algorithm: Option<String>,
    /// TALA's caller options (seeds), as passed to the layout.
    pub(crate) tala: d2_rs_tala::LayoutOptions,
}

/// Writes the LayoutNested records of every board that d2lib.Compile lays
/// out, in Go's order: the root, then each board's layers, scenarios and
/// steps (d2.go:233-260). `engine` is the CompileOptions.Layout string.
/// Returns false when Go's compile stops at a board (the records end there
/// and d2cli.compile logs nothing further).
pub(crate) fn emit_compile_trace(
    compiler: &d2_rs_language::compiler::Compiler,
    root: d2_rs_core::ir::NodeId,
    engine: &str,
    checks: &CoreChecks,
    lower: &mut LowerBoard<'_>,
) -> bool {
    let boards = board_tree(compiler, root);
    compile_board(&boards, 0, None, false, engine, checks, lower).is_ok()
}

fn board_tree(
    compiler: &d2_rs_language::compiler::Compiler,
    root: d2_rs_core::ir::NodeId,
) -> Vec<Board> {
    let mut boards = vec![Board {
        node: root,
        path: String::new(),
        kind: BoardKind::Layer,
        children: Vec::new(),
    }];
    let mut stack = vec![0];
    // board_targets is preorder with layers, scenarios, then steps.
    for (path, node) in d2_rs_language::compiler::board_targets(compiler, root) {
        while stack.len() > 1 {
            let top = &boards[*stack.last().unwrap()].path;
            if path.len() > top.len() + 1
                && path.starts_with(top.as_str())
                && path.as_bytes()[top.len()] == b'.'
            {
                break;
            }
            stack.pop();
        }
        let parent = *stack.last().unwrap();
        let rest = if boards[parent].path.is_empty() {
            path.as_str()
        } else {
            &path[boards[parent].path.len() + 1..]
        };
        let kind = match rest.split('.').next() {
            Some("layers") => BoardKind::Layer,
            Some("scenarios") => BoardKind::Scenario,
            _ => BoardKind::Step,
        };
        let index = boards.len();
        boards.push(Board {
            node,
            path,
            kind,
            children: Vec::new(),
        });
        boards[parent].children.push(index);
        stack.push(index);
    }
    boards
}

/// d2lib compileBoard (d2.go:184-264). Returns the board's captured layout
/// geometry, represented by its layout-input signature.
fn compile_board(
    boards: &[Board],
    index: usize,
    reuse: Option<&serde_json::Value>,
    capture_for_caller: bool,
    engine: &str,
    checks: &CoreChecks,
    lower: &mut LowerBoard<'_>,
) -> Result<Option<serde_json::Value>, ()> {
    let board = &boards[index];
    let graph = lower(board.node).map_err(|_| ())?;
    let has_scenarios_or_steps = board
        .children
        .iter()
        .any(|child| boards[*child].kind != BoardKind::Layer);
    let mut geometry = None;
    if !graph.nodes.is_empty() {
        let needs_geometry = reuse.is_some() || capture_for_caller || has_scenarios_or_steps;
        let signature = if needs_geometry {
            layout_input_signature(&graph, engine)
        } else {
            None
        };
        // layoutGeometry.apply: same signature (object and edge IDs included).
        let reused = signature.is_some() && reuse.is_some() && signature.as_ref() == reuse;
        if !reused {
            Trace::new(&graph, checks).layout_board()?;
        }
        if signature.is_some() && (capture_for_caller || has_scenarios_or_steps) {
            geometry = signature;
        }
    }
    for child in &board.children {
        if boards[*child].kind == BoardKind::Layer {
            compile_board(boards, *child, None, false, engine, checks, lower)?;
        }
    }
    for child in &board.children {
        if boards[*child].kind == BoardKind::Scenario {
            compile_board(
                boards,
                *child,
                geometry.as_ref(),
                false,
                engine,
                checks,
                lower,
            )?;
        }
    }
    let steps = board
        .children
        .iter()
        .copied()
        .filter(|child| boards[*child].kind == BoardKind::Step)
        .collect::<Vec<_>>();
    let mut previous = geometry.clone();
    for (position, child) in steps.iter().enumerate() {
        previous = compile_board(
            boards,
            *child,
            previous.as_ref(),
            position + 1 < steps.len(),
            engine,
            checks,
            lower,
        )?;
    }
    Ok(geometry)
}

/// d2lib newLayoutInputSignature / supportsLayoutReuse (layout_reuse.go:28-74).
/// Only the built-in Dagre and ELK names (exact spelling) reuse geometry, and
/// never with sequence, grid or constant-near objects. The value lists the
/// fields layoutSignatureBuilder writes, in its order (writeObject :76-108,
/// writeEdge :110-134, writeAttributes :136-162), read from the compiler's
/// d2graph-shaped source records; layoutGeometry.apply (:322-346) reuses on
/// equality, so only the recorded information matters, not its byte form.
fn layout_input_signature(graph: &Graph, engine: &str) -> Option<serde_json::Value> {
    use serde_json::{Value, json};
    if engine != "dagre" && engine != "elk" {
        return None;
    }
    if graph.root_sequence_diagram
        || graph.root_grid_rows.is_some()
        || graph.root_grid_columns.is_some()
    {
        return None;
    }
    for (id, node) in graph.nodes() {
        if graph.is_sequence_diagram(id)
            || node.grid_rows.is_some()
            || node.grid_columns.is_some()
            || node.canvas_position.is_some()
        {
            return None;
        }
    }
    let metadata = graph.source_metadata.as_ref()?;
    let scalar =
        |attributes: &Value, key: &str| attributes.get(key).map(|value| value["value"].clone());
    // d2graph.Key(NearKey): the key path's strings, without source ranges.
    fn without_ranges(value: &Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .filter(|(key, _)| key.as_str() != "range")
                    .map(|(key, value)| (key.clone(), without_ranges(value)))
                    .collect(),
            ),
            Value::Array(values) => Value::Array(values.iter().map(without_ranges).collect()),
            other => other.clone(),
        }
    }
    let attributes = |attributes: &Value| {
        let dimension = |key: &str| {
            attributes["labelDimensions"][key]
                .as_f64()
                .map(|value| value as i64)
        };
        json!([
            attributes["label"]["value"]
                .as_str()
                .is_some_and(|label| !label.is_empty()),
            dimension("width"),
            dimension("height"),
            attributes["shape"]["value"],
            attributes["direction"]["value"],
            attributes["constraint"],
            attributes.get("language"),
            scalar(attributes, "width"),
            scalar(attributes, "height"),
            scalar(attributes, "top"),
            scalar(attributes, "left"),
            scalar(attributes, "gridRows"),
            scalar(attributes, "gridColumns"),
            scalar(attributes, "gridGap"),
            scalar(attributes, "verticalGap"),
            scalar(attributes, "horizontalGap"),
            scalar(attributes, "labelPosition"),
            scalar(attributes, "iconPosition"),
            without_ranges(&attributes["near_key"]),
        ])
    };
    let children = |parent: Option<d2_rs_core::NodeId>| -> Vec<String> {
        match graph.hierarchy_children_order.get(&parent) {
            Some(children) => children
                .iter()
                .filter_map(|child| graph.node(*child))
                .map(|child| child.external_id.clone())
                .collect(),
            None => graph
                .nodes()
                .filter(|(_, node)| node.parent == parent)
                .map(|(_, node)| node.external_id.clone())
                .collect(),
        }
    };
    let object = |id: Option<d2_rs_core::NodeId>, value: &Value| {
        let attrs = &value["attributes"];
        let shape = attrs["shape"]["value"].as_str().unwrap_or("");
        let has_label = !matches!(shape, "text" | "class" | "sql_table" | "code")
            && attrs["label"]["value"]
                .as_str()
                .is_some_and(|label| !label.is_empty());
        let icon = attrs.get("icon").is_some_and(|icon| !icon.is_null());
        let style_true = |key: &str| attrs["style"][key]["value"] == "true";
        let children = children(id);
        let (abs_id, parent, size) = match id.and_then(|id| graph.node(id).map(|node| (id, node))) {
            Some((_, node)) => (
                node.external_id.clone(),
                Some(
                    node.parent
                        .and_then(|parent| graph.node(parent))
                        .map(|parent| parent.external_id.clone())
                        .unwrap_or_default(),
                ),
                // SetDimensions: Box{} with Width/Height, no TopLeft.
                json!([node.size.width, node.size.height]),
            ),
            None => (String::new(), None, Value::Null),
        };
        // SetDimensions converts the label/icon position attributes for
        // objects only; the root's stay unset before layout.
        let label_position = (id.is_some() && has_label)
            .then(|| scalar(attrs, "labelPosition"))
            .flatten();
        let icon_position = (id.is_some() && icon)
            .then(|| scalar(attrs, "iconPosition"))
            .flatten();
        json!([
            abs_id,
            parent,
            children.len(),
            children,
            !children.is_empty(),
            size,
            label_position,
            icon_position,
            value.get("contentAspectRatio"),
            has_label,
            icon && shape != "image",
            icon,
            style_true("3d"),
            style_true("multiple"),
            value.get("class"),
            value.get("sql_table"),
            attributes(attrs),
        ])
    };
    let root = graph.source_root.clone().unwrap_or(Value::Null);
    let mut objects = vec![object(None, &root)];
    let mut order = graph.node_order_override.clone();
    for (id, _) in graph.nodes() {
        if !order.contains(&id) {
            order.push(id);
        }
    }
    for id in order {
        objects.push(object(Some(id), &metadata.objects.get(&id)?.value));
    }
    let mut edges = Vec::new();
    for (id, edge) in graph.edges() {
        let value = metadata.edges.get(&id)?;
        let endpoint =
            |node: d2_rs_core::NodeId| graph.node(node).map(|node| node.external_id.clone());
        let columns = graph
            .edge_table_columns
            .get(id.0 as usize)
            .copied()
            .unwrap_or_default();
        let arrows = graph
            .edge_arrows
            .get(id.0 as usize)
            .copied()
            .unwrap_or_default();
        let arrowhead = |key: &str| value.get(key).map(attributes);
        edges.push(json!([
            d2_rs_core::edge_abs_id(graph, id),
            endpoint(edge.source),
            endpoint(edge.target),
            graph
                .edge_indices
                .get(id.0 as usize)
                .copied()
                .flatten()
                .unwrap_or(id.0 as usize),
            columns.source,
            columns.target,
            arrows.source,
            arrows.target,
            graph.edge_is_curve(id),
            // Route, LabelPosition and LabelPercentage are unset before layout.
            attributes(&value["attributes"]),
            arrowhead("srcArrowhead"),
            arrowhead("dstArrowhead"),
        ]));
    }
    Some(json!([engine, 0, objects, edges]))
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum DiagramType {
    #[default]
    Default,
    Grid,
    Sequence,
}

/// d2layouts.GraphInfo; `%v` prints `{IsConstantNear DiagramType}`.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct GraphInfo {
    is_constant_near: bool,
    diagram_type: DiagramType,
}

impl GraphInfo {
    fn is_default(self) -> bool {
        !self.is_constant_near && self.diagram_type == DiagramType::Default
    }
}

impl std::fmt::Display for GraphInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let diagram_type = match self.diagram_type {
            DiagramType::Default => "",
            DiagramType::Grid => "grid-diagram",
            DiagramType::Sequence => "sequence-diagram",
        };
        write!(f, "{{{} {diagram_type}}}", self.is_constant_near)
    }
}

/// A d2graph.Object: its ID, tree links and the attributes LayoutNested reads.
struct Object {
    id: String,
    parent: Option<usize>,
    children: Vec<usize>,
    graph: usize,
    sequence: bool,
    grid: bool,
    near: Option<CanvasPosition>,
    /// LayoutNested clears NearKey while laying out a constant-near graph.
    near_cleared: bool,
}

/// A d2graph.Graph: root object, Objects order and RootLevel.
struct SimGraph {
    root: usize,
    objects: Vec<usize>,
    root_level: i64,
    /// The measured-graph node whose attributes the root carries (`None`
    /// for the board root), to check a sequence scope's actors.
    scope: Option<d2_rs_core::NodeId>,
}

struct Trace<'a> {
    graph: &'a Graph,
    checks: &'a CoreChecks,
    objects: Vec<Object>,
    graphs: Vec<SimGraph>,
}

/// A d2layouts subgraphExtractionSpec.
struct ExtractionSpec {
    container: usize,
    include_self: bool,
    info: GraphInfo,
}

impl<'a> Trace<'a> {
    fn new(graph: &'a Graph, checks: &'a CoreChecks) -> Self {
        let mut objects = vec![Object {
            id: String::new(),
            parent: None,
            children: Vec::new(),
            graph: 0,
            sequence: graph.root_sequence_diagram,
            grid: graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some(),
            near: None,
            near_cleared: false,
        }];
        for (id, node) in graph.nodes() {
            let parent_id = node
                .parent
                .and_then(|parent| graph.node(parent))
                .map(|parent| parent.external_id.as_str());
            let own_id = match parent_id {
                Some(parent) => node
                    .external_id
                    .strip_prefix(parent)
                    .and_then(|rest| rest.strip_prefix('.'))
                    .unwrap_or(&node.external_id),
                None => &node.external_id,
            };
            objects.push(Object {
                id: own_id.to_owned(),
                parent: Some(node.parent.map_or(0, |parent| parent.0 as usize + 1)),
                children: Vec::new(),
                graph: 0,
                sequence: graph.is_sequence_diagram(id),
                grid: node.grid_rows.is_some() || node.grid_columns.is_some(),
                near: node.canvas_position,
                near_cleared: false,
            });
        }
        // Object.ChildrenArray, in the compiler's child creation order.
        if graph.hierarchy_children_order.is_empty() {
            for index in 1..objects.len() {
                let parent = objects[index].parent.unwrap();
                objects[parent].children.push(index);
            }
        } else {
            for (parent, children) in &graph.hierarchy_children_order {
                let parent = parent.map_or(0, |parent| parent.0 as usize + 1);
                objects[parent].children = children
                    .iter()
                    .map(|child| child.0 as usize + 1)
                    .filter(|child| *child < objects.len())
                    .collect();
            }
        }
        // Graph.Objects is in SortObjectsByAST order (the compiler's
        // node_order_override); ChildrenArray keeps creation order.
        let count = objects.len();
        let mut order = Vec::with_capacity(count - 1);
        let mut seen = vec![false; count];
        for node in &graph.node_order_override {
            let index = node.0 as usize + 1;
            if index < count && !std::mem::replace(&mut seen[index], true) {
                order.push(index);
            }
        }
        order.extend((1..count).filter(|index| !seen[*index]));
        Self {
            graph,
            checks,
            objects,
            graphs: vec![SimGraph {
                root: 0,
                objects: order,
                root_level: 0,
                scope: None,
            }],
        }
    }

    /// d2lib compileBoard: LayoutNested(g, NestedGraphInfo(g.Root)).
    fn layout_board(mut self) -> Result<(), ()> {
        let info = self.nested_graph_info(0);
        self.layout_nested(0, info)
    }

    /// Object.AbsID.
    fn abs_id(&self, object: usize) -> String {
        let mut parts = Vec::new();
        let mut current = object;
        loop {
            let entry = &self.objects[current];
            parts.push(entry.id.as_str());
            match entry.parent {
                Some(parent) if !self.objects[parent].id.is_empty() => current = parent,
                _ => break,
            }
        }
        parts.reverse();
        parts.join(".")
    }

    /// Object.Level.
    fn level(&self, object: usize) -> i64 {
        match self.objects[object].parent {
            None => self.graphs[self.objects[object].graph].root_level,
            Some(parent) => 1 + self.level(parent),
        }
    }

    fn is_descendant_of(&self, object: usize, ancestor: usize) -> bool {
        let mut current = Some(object);
        while let Some(object) = current {
            if object == ancestor {
                return true;
            }
            current = self.objects[object].parent;
        }
        false
    }

    /// d2layouts.NestedGraphInfo.
    fn nested_graph_info(&self, object: usize) -> GraphInfo {
        let entry = &self.objects[object];
        GraphInfo {
            is_constant_near: self.graphs[entry.graph].root_level == 0
                && entry.near.is_some()
                && !entry.near_cleared,
            diagram_type: if entry.sequence {
                DiagramType::Sequence
            } else if entry.grid {
                DiagramType::Grid
            } else {
                DiagramType::Default
            },
        }
    }

    /// Whether coreLayout(ctx, g) returns an error for this graph's objects.
    fn core_layout_fails(&self, graph: usize) -> bool {
        let live = self.graphs[graph]
            .objects
            .iter()
            .map(|object| d2_rs_core::NodeId(*object as u32 - 1))
            .collect::<Vec<_>>();
        match self.checks.engine.as_str() {
            "tala" => d2_rs_layout::scope_checks::validate_tala_scope(
                self.graph,
                &live,
                &self.checks.tala,
            )
            .is_err(),
            "elk" => self.checks.elk_algorithm.as_deref().is_some_and(|name| {
                d2_rs_layout::scope_checks::validate_elk_algorithm(name).is_err()
            }),
            _ => false,
        }
    }

    /// Graph.PrintString.
    fn print_string(&self, graph: usize) -> String {
        let mut text = String::from("Objects: [");
        for object in &self.graphs[graph].objects {
            text.push_str(&self.abs_id(*object));
            text.push_str(", ");
        }
        text.push(']');
        text
    }

    fn new_graph(&mut self, container: usize, root_level: i64) -> usize {
        let graph = self.graphs.len();
        let root = self.objects.len();
        let container_index = container;
        // nestedGraph.Root.Attributes = container.Attributes.
        let container = &self.objects[container];
        let root_object = Object {
            id: String::new(),
            parent: None,
            children: Vec::new(),
            graph,
            sequence: container.sequence,
            grid: container.grid,
            near: container.near,
            near_cleared: false,
        };
        self.objects.push(root_object);
        self.graphs.push(SimGraph {
            root,
            objects: Vec::new(),
            root_level,
            // Extracted containers are always source objects (arena index
            // minus the board root at 0).
            scope: Some(d2_rs_core::NodeId(container_index as u32 - 1)),
        });
        graph
    }

    fn remove_child(&mut self, parent: usize, child: usize) {
        self.objects[parent]
            .children
            .retain(|candidate| *candidate != child);
    }

    /// Moves the container (includeSelf) or its children under the nested root.
    fn reroot(&mut self, container: usize, nested: usize, include_self: bool) {
        let root = self.graphs[nested].root;
        if include_self {
            if let Some(parent) = self.objects[container].parent {
                self.remove_child(parent, container);
            }
            self.objects[root].children = vec![container];
            self.objects[container].parent = Some(root);
        } else {
            let children = std::mem::take(&mut self.objects[container].children);
            for child in &children {
                self.objects[*child].parent = Some(root);
            }
            self.objects[root].children.extend(children);
        }
    }

    /// d2layouts.ExtractSubgraph (d2layouts.go:625-722).
    fn extract_subgraph(&mut self, container: usize, include_self: bool) -> usize {
        let root_level = self.level(container) - i64::from(include_self);
        let nested = self.new_graph(container, root_level);
        let graph = self.objects[container].graph;
        let mut remaining = Vec::new();
        let mut moved = Vec::new();
        for object in std::mem::take(&mut self.graphs[graph].objects) {
            let is_nested = if include_self {
                self.is_descendant_of(object, container)
            } else {
                self.objects[object]
                    .parent
                    .is_some_and(|parent| self.is_descendant_of(parent, container))
            };
            if is_nested {
                moved.push(object);
            } else {
                remaining.push(object);
            }
        }
        self.graphs[graph].objects = remaining;
        for object in &moved {
            self.objects[*object].graph = nested;
        }
        self.graphs[nested].objects = moved;
        self.reroot(container, nested, include_self);
        nested
    }

    /// d2layouts.collectNestedExtractions (d2layouts.go:428-452).
    fn collect_nested_extractions(&self, graph: usize) -> Vec<ExtractionSpec> {
        let root = self.graphs[graph].root;
        let mut queue = self.objects[root]
            .children
            .iter()
            .copied()
            .collect::<VecDeque<_>>();
        let mut specs = Vec::new();
        while let Some(current) = queue.pop_front() {
            let info = self.nested_graph_info(current);
            if !info.is_default() {
                if !info.is_constant_near && self.objects[current].children.is_empty() {
                    continue;
                }
                specs.push(ExtractionSpec {
                    container: current,
                    include_self: info.is_constant_near,
                    info,
                });
                continue;
            }
            queue.extend(self.objects[current].children.iter().copied());
        }
        specs
    }

    /// d2layouts.extractSubgraphs (d2layouts.go:457-583): one partition pass,
    /// then the same re-rooting as sequential ExtractSubgraph calls.
    fn extract_subgraphs(
        &mut self,
        graph: usize,
        specs: &[ExtractionSpec],
    ) -> HashMap<usize, (usize, GraphInfo)> {
        let mut owners = HashMap::new();
        let mut nested_graphs = Vec::new();
        for (index, spec) in specs.iter().enumerate() {
            owners.insert(spec.container, index);
            let root_level = self.level(spec.container) - i64::from(spec.include_self);
            nested_graphs.push(self.new_graph(spec.container, root_level));
        }
        let find_owner = |trace: &Self, object: usize| {
            let mut ancestor = Some(object);
            while let Some(current) = ancestor {
                if let Some(owner) = owners.get(&current) {
                    if specs[*owner].include_self || current != object {
                        return Some(*owner);
                    }
                    break;
                }
                ancestor = trace.objects[current].parent;
            }
            None
        };
        let mut remaining = Vec::new();
        for object in self.graphs[graph].objects.clone() {
            match find_owner(self, object) {
                Some(owner) => self.graphs[nested_graphs[owner]].objects.push(object),
                None => remaining.push(object),
            }
        }
        self.graphs[graph].objects = remaining;
        let mut extractions = HashMap::new();
        for (index, spec) in specs.iter().enumerate() {
            let nested = nested_graphs[index];
            for object in self.graphs[nested].objects.clone() {
                self.objects[object].graph = nested;
            }
            self.reroot(spec.container, nested, spec.include_self);
            extractions.insert(spec.container, (nested, spec.info));
        }
        extractions
    }

    /// d2layouts.InjectNested (d2layouts.go:724-744).
    fn inject_nested(&mut self, container: usize, nested: usize) {
        let root = self.graphs[nested].root;
        let graph = self.objects[container].graph;
        for child in self.objects[root].children.clone() {
            self.objects[child].parent = Some(container);
            self.objects[container].children.push(child);
        }
        let objects = self.graphs[nested].objects.clone();
        for object in &objects {
            self.objects[*object].graph = graph;
        }
        self.graphs[graph].objects.extend(objects);
    }

    /// d2near.Layout's object moves (d2near/layout.go:35-87): constant-near
    /// graphs rejoin in vertical-center, horizontal-center, corner order.
    fn layout_constant_nears(&mut self, graph: usize, nears: &[usize]) {
        let root = self.graphs[graph].root;
        for near in nears {
            let object = self.objects[self.graphs[*near].root].children[0];
            self.objects[object].parent = Some(root);
            for member in self.graphs[*near].objects.clone() {
                self.objects[member].graph = graph;
            }
        }
        let sets: [&[CanvasPosition]; 3] = [
            &[CanvasPosition::TopCenter, CanvasPosition::BottomCenter],
            &[CanvasPosition::CenterLeft, CanvasPosition::CenterRight],
            &[
                CanvasPosition::TopLeft,
                CanvasPosition::TopRight,
                CanvasPosition::BottomLeft,
                CanvasPosition::BottomRight,
            ],
        ];
        for set in sets {
            for near in nears {
                let object = self.objects[self.graphs[*near].root].children[0];
                if self.objects[object]
                    .near
                    .is_some_and(|key| set.contains(&key))
                {
                    let members = self.graphs[*near].objects.clone();
                    self.graphs[graph].objects.extend(members);
                    let parent = self.objects[object].parent.unwrap_or(root);
                    self.objects[parent].children.push(object);
                }
            }
        }
    }

    /// SaveOrder's restore closure: stable sorts by the saved AbsID index.
    fn restore_order(
        &mut self,
        graph: usize,
        objects: &HashMap<String, usize>,
        children: &HashMap<String, usize>,
    ) {
        let mut keyed = self.graphs[graph]
            .objects
            .iter()
            .map(|object| {
                (
                    objects.get(&self.abs_id(*object)).copied().unwrap_or(0),
                    *object,
                )
            })
            .collect::<Vec<_>>();
        keyed.sort_by_key(|(key, _)| *key);
        self.graphs[graph].objects = keyed.into_iter().map(|(_, object)| object).collect();
        let root = self.graphs[graph].root;
        let mut keyed = self.objects[root]
            .children
            .iter()
            .map(|object| {
                (
                    children.get(&self.abs_id(*object)).copied().unwrap_or(0),
                    *object,
                )
            })
            .collect::<Vec<_>>();
        keyed.sort_by_key(|(key, _)| *key);
        self.objects[root].children = keyed.into_iter().map(|(_, object)| object).collect();
    }

    fn order_index(&self, objects: &[usize]) -> HashMap<String, usize> {
        objects
            .iter()
            .enumerate()
            .map(|(index, object)| (self.abs_id(*object), index))
            .collect()
    }

    /// d2layouts.LayoutNested (d2layouts.go:78-373), keeping only the object
    /// moves and the log.Debug records.
    fn layout_nested(&mut self, graph: usize, info: GraphInfo) -> Result<(), ()> {
        let root = self.graphs[graph].root;
        let saved_objects = self.order_index(&self.graphs[graph].objects.clone());
        let saved_children = self.order_index(&self.objects[root].children.clone());
        let mut extracted = HashMap::new();
        let mut extracted_order = Vec::new();
        let mut constant_nears = Vec::new();
        let mut queue = self.objects[root]
            .children
            .iter()
            .copied()
            .collect::<VecDeque<_>>();
        let mut batched = HashMap::new();
        if info.diagram_type != DiagramType::Grid {
            let specs = self.collect_nested_extractions(graph);
            if specs.len() > 1 {
                batched = self.extract_subgraphs(graph, &specs);
                queue = specs.iter().map(|spec| spec.container).collect();
            }
        }
        while let Some(mut current) = queue.pop_front() {
            let is_grid_cell_container = info.diagram_type == DiagramType::Grid
                && !self.objects[current].children.is_empty()
                && self.objects[current].parent == Some(root);
            let mut current_info = self.nested_graph_info(current);
            if let Some((_, batched_info)) = batched.get(&current) {
                current_info = *batched_info;
            }
            if is_grid_cell_container && current_info.is_default() {
                let nested = self.extract_subgraph(current, true);
                self.layout_nested(nested, GraphInfo::default())?;
                self.inject_nested(root, nested);
                self.restore_order(graph, &saved_objects, &saved_children);
                // Then curr's contents are extracted again (includeSelf=false)
                // and restored after the grid layout (d2layouts.go:187-207).
                let nested = self.extract_subgraph(current, false);
                let id = self.abs_id(current);
                extracted.insert(id.clone(), nested);
                extracted_order.push(id);
                continue;
            }
            if !current_info.is_default() {
                let was_batched = batched.contains_key(&current);
                if !was_batched
                    && !current_info.is_constant_near
                    && self.objects[current].children.is_empty()
                {
                    continue;
                }
                let nested = match batched.get(&current) {
                    Some((nested, _)) => *nested,
                    None => self.extract_subgraph(current, current_info.is_constant_near),
                };
                cli_log::debug(&format!(
                    "layout nested level={} child={} gi={current_info}",
                    self.level(current),
                    self.abs_id(current)
                ));
                let mut nested_info = current_info;
                if current_info.is_constant_near {
                    nested_info = GraphInfo::default();
                    self.objects[current].near_cleared = true;
                }
                self.layout_nested(nested, nested_info)?;
                if current_info.is_constant_near {
                    current = self.objects[self.graphs[nested].root].children[0];
                    self.objects[current].near_cleared = false;
                    constant_nears.push(nested);
                } else {
                    let id = self.abs_id(current);
                    extracted.insert(id.clone(), nested);
                    extracted_order.push(id);
                }
            } else if !self.objects[current].children.is_empty() {
                queue.extend(self.objects[current].children.iter().copied());
            }
        }
        if !self.graphs[graph].objects.is_empty() {
            let message = match info.diagram_type {
                DiagramType::Grid => "layout grid",
                DiagramType::Sequence => "layout sequence",
                DiagramType::Default => "default layout",
            };
            cli_log::debug(&format!(
                "{message} rootlevel={} shapes={}",
                self.graphs[graph].root_level,
                self.print_string(graph)
            ));
            if info.diagram_type == DiagramType::Default && self.core_layout_fails(graph) {
                return Err(());
            }
            // d2sequence.Layout fails in newSequenceDiagram before laying
            // out; Go's compile stops after this record.
            if info.diagram_type == DiagramType::Sequence
                && d2_rs_layout::sequence::validate_sequence_actors(
                    self.graph,
                    self.graphs[graph].scope,
                    &self.graphs[graph]
                        .objects
                        .iter()
                        .map(|object| d2_rs_core::NodeId(*object as u32 - 1))
                        .collect::<Vec<_>>(),
                )
                .is_err()
            {
                return Err(());
            }
        }
        if !constant_nears.is_empty() {
            self.layout_constant_nears(graph, &constant_nears);
        }
        for id in extracted_order {
            let nested = extracted[&id];
            let object = self.graphs[graph]
                .objects
                .iter()
                .copied()
                .find(|object| self.abs_id(*object) == id)
                .ok_or(())?;
            self.inject_nested(object, nested);
        }
        cli_log::debug(&format!(
            "done rootlevel={} shapes={}",
            self.graphs[graph].root_level,
            self.print_string(graph)
        ));
        self.restore_order(graph, &saved_objects, &saved_children);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_info_prints_like_go_struct_value() {
        assert_eq!(GraphInfo::default().to_string(), "{false }");
        assert_eq!(
            GraphInfo {
                is_constant_near: true,
                diagram_type: DiagramType::Default
            }
            .to_string(),
            "{true }"
        );
        assert_eq!(
            GraphInfo {
                is_constant_near: false,
                diagram_type: DiagramType::Sequence
            }
            .to_string(),
            "{false sequence-diagram}"
        );
    }
}
