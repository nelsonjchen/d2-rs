// SPDX-License-Identifier: MPL-2.0
//! Pre-measurement d2graph projection from pinned d2compiler/compile.go.
use super::{CompileContext, Compiler, GraphCompileError, graph_materialization::GraphContext};
use d2_rs_core::{Size, ast, ir};
use serde_json::{Value, json};

/// Construct the original compiler graph contract before SetDimensions.
/// BaseAST materialization intentionally updates the supplied source AST,
/// as d2compiler.findFieldAST does for imported and scalar boards.
pub fn source_graph_json(
    compiler: &Compiler,
    root: ir::NodeId,
    ast: ast::Shared<ast::Map>,
) -> Result<Value, GraphCompileError> {
    source_graph_view(compiler, root, ast, None)
}

/// Materialize complete source graphs with a graph-stage cooperative context.
/// Shares the IR compilation's copy budget across every board and repeated calls.
/// Errors publish no partial graph; cancellation retains its typed variant.
pub fn source_graph_json_with_context(
    compiler: &Compiler,
    root: ir::NodeId,
    ast: ast::Shared<ast::Map>,
    context: Option<CompileContext>,
) -> Result<Value, GraphCompileError> {
    source_graph_view(compiler, root, ast, context)
}

fn source_graph_view(
    compiler: &Compiler,
    root: ir::NodeId,
    ast: ast::Shared<ast::Map>,
    context: Option<CompileContext>,
) -> Result<Value, GraphCompileError> {
    let mut view = Compiler {
        arena: compiler.arena.fork_storage(),
        errors: compiler.errors.clone(),
        variable_expansion: compiler.variable_expansion.clone(),
        graph_context: Some(std::rc::Rc::new(std::cell::RefCell::new(
            GraphContext::new(context),
        ))),
        ..Default::default()
    };
    view.graph_check().and_then(|_| {
        compile_board(
            &mut view,
            root,
            Some(ast::Child::Map(ast.clone())),
            Some(ast),
            "",
        )
    })
}

/// Find a board's writable source map, preserving imports as spread nodes.
/// Maps d2compiler._findFieldAST (compile.go:230–267).
pub fn find_board_base_ast(
    source: &ast::Shared<ast::Map>,
    path: &[&str],
) -> Option<ast::Shared<ast::Map>> {
    if path.is_empty() {
        return Some(source.clone());
    }
    let nodes = source.read().nodes.clone();
    for slot in nodes.iter().flat_map(ast::SharedSlice::iter) {
        let Some(key) = slot.read().map_key.clone() else {
            continue;
        };
        let parts = key
            .read()
            .key
            .as_ref()
            .and_then(|p| p.read().string_ida())
            .unwrap_or_default();
        if parts.len() != 1 || parts[0] != path[0] {
            continue;
        }
        if key.read().value.map.is_none() {
            let mut range = ast::Range::default();
            range.start.line = 1;
            range.end.line = 1;
            let nodes = key.read().value.import.as_ref().map(|import| {
                let mut import = import.read().clone();
                import.range = range.clone();
                import.spread = true;
                vec![ast::MapNodeBox {
                    import: Some(ast::Shared::new(import)),
                    ..Default::default()
                }]
                .into()
            });
            key.write().value.map = Some(ast::Shared::new(ast::Map { range, nodes }));
        }
        let map = key.read().value.map.clone().unwrap();
        if let Some(found) = find_board_base_ast(&map, &path[1..]) {
            return Some(found);
        }
    }
    None
}

fn compile_board(
    compiler: &mut Compiler,
    root: ir::NodeId,
    source: Option<ast::Child>,
    base: Option<ast::Shared<ast::Map>>,
    name: &str,
) -> Result<Value, GraphCompileError> {
    let root = compiler.prepare_graph_board(root)?;
    compiler.graph_class_root = Some(root);
    let (graph, _, _) =
        super::lower_graph_with_measurer_and_labels(compiler, root, &|_: &str| Size::default())?;
    let mut value = graph_json(&graph);
    value["name"] = json!(name);
    if !name.is_empty() {
        if let Some(label) = super::board_label(compiler, root) {
            value["root"]["attributes"]["label"] = json!({"value":label});
        }
    }
    for wrapper in ["layers", "scenarios", "steps"] {
        let Some(map) = compiler
            .arena
            .get_field(root, &[ast::raw_string_box(wrapper, true)])
            .and_then(|id| compiler.arena.map(id))
        else {
            continue;
        };
        let ir::Node::Map(map) = compiler.arena.node(map) else {
            unreachable!()
        };
        let fields = map.fields.clone().unwrap_or_default();
        let mut children = Vec::new();
        for field in fields {
            compiler.graph_check()?;
            let ir::Node::Field(f) = compiler.arena.node(field) else {
                unreachable!()
            };
            let name = f.name.scalar_string();
            let label =
                compiler
                    .arena
                    .primary(field)
                    .and_then(|id| match compiler.arena.node(id) {
                        ir::Node::Scalar(s) => Some(s.value.scalar_string()),
                        _ => None,
                    });
            let board = compiler
                .arena
                .map(field)
                .unwrap_or_else(|| compiler.arena.new_root());
            let source = compiler.arena.ast(board);
            let child_base = base
                .as_ref()
                .and_then(|b| find_board_base_ast(b, &[wrapper, &name]));
            let mut child = compile_board(compiler, board, source, child_base, &name)?;
            if let Some(label) = label {
                child["root"]["attributes"]["label"] = json!({"value":label});
            }
            children.push(child);
        }
        if !children.is_empty() {
            value[wrapper] = json!(children);
        }
    }
    // Source compileBoard's folder comparison makes a second automatic copy.
    if ["layers", "scenarios", "steps"]
        .iter()
        .any(|key| value.get(*key).is_some())
    {
        compiler.reserve_graph_copy(root)?;
    }
    compiler.graph_check()?;
    value["ast"] = json!(source);
    Ok(value)
}

fn graph_json(graph: &d2_rs_core::Graph) -> Value {
    let empty = d2_rs_core::SourceGraphMetadata::default();
    let metadata = graph.source_metadata.as_deref().unwrap_or(&empty);
    let order = if graph.node_order_override.is_empty() {
        graph.nodes().map(|(id, _)| id).collect::<Vec<_>>()
    } else {
        graph.node_order_override.clone()
    };
    let mut objects = order
        .iter()
        .map(|id| metadata.objects[id].clone())
        .collect::<Vec<_>>();
    let mut edges = graph
        .edges()
        .map(|(id, _)| {
            let mut value = metadata.edges[&id].clone();
            value["index"] = json!(
                graph
                    .edge_indices
                    .get(id.0 as usize)
                    .copied()
                    .flatten()
                    .unwrap_or(0)
            );
            value["isCurve"] = json!(graph.edge_is_curve(id));
            let arrows = graph.edge_arrows(id);
            value["src_arrow"] = json!(arrows.source);
            value["dst_arrow"] = json!(arrows.target);
            let columns = graph.edge_table_columns(id);
            if let Some(column) = columns.source {
                value["srcTableColumnIndex"] = json!(column);
            }
            if let Some(column) = columns.target {
                value["dstTableColumnIndex"] = json!(column);
            }
            value
        })
        .collect::<Vec<_>>();
    // This adapter constructs the pre-SetDimensions compiler contract from
    // the native graph's source records, not its later measured geometry.
    for value in objects.iter_mut().chain(edges.iter_mut()) {
        value["attributes"]["labelDimensions"] = json!({"width":0,"height":0});
        for key in ["srcArrowhead", "dstArrowhead"] {
            if let Some(attrs) = value.get_mut(key) {
                attrs["labelDimensions"] = json!({"width":0,"height":0});
            }
        }
    }
    let mut value = json!({"name":"","isFolderOnly":objects.is_empty(),"ast":null,
        "root":graph.source_root,
        "objects":if objects.is_empty() && !metadata.objects_allocated {Value::Null} else {json!(objects)},
        "edges":if edges.is_empty() {Value::Null} else {json!(edges)}});
    if let Some(legend) = graph
        .legend
        .as_ref()
        .and_then(|legend| legend.source_graph.as_deref())
    {
        let mut record = graph_json(&legend.graph);
        if let Some(objects) = record["objects"].as_array_mut() {
            objects.retain(|v| {
                v["attributes"]["style"]["opacity"]["value"]
                    .as_str()
                    .and_then(d2_rs_core::parse_d2_float)
                    != Some(0.0)
            });
            for object in objects {
                object["box"] = json!({"TopLeft":{"x":10,"y":10},"Width":100,"Height":100});
            }
        }
        if let Some(edges) = record["edges"].as_array_mut() {
            for edge in edges {
                edge["route"] = json!([{"x":10,"y":10},{"x":110,"y":10}]);
            }
        }
        let mut result = json!({});
        if !legend.label.is_empty() {
            result["label"] = json!(legend.label);
        }
        if record["objects"].as_array().is_some_and(|v| !v.is_empty()) {
            result["objects"] = record["objects"].clone();
        }
        if record["edges"].as_array().is_some_and(|v| !v.is_empty()) {
            result["edges"] = record["edges"].clone();
        }
        value["legend"] = result;
    }
    value
}
