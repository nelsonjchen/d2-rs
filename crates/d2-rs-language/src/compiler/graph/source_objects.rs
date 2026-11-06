// SPDX-License-Identifier: MPL-2.0
//! Original compiled objects/references, retained before layout ownership changes.
use super::*;
use serde_json::{Value, json};

/// d2compiler retains KeyPath pointers in references until JSON is requested.
/// Snapshot each allocation once per lowering, after validation. Keeping the
/// handle also prevents pointer reuse while this memo is alive; the resulting
/// graph owns only immutable, thread-safe snapshots.
#[derive(Default)]
pub(super) struct KeyPool {
    // Render/plan projections never insert keys. An empty ordered map needs
    // neither allocation nor RandomState initialization on those hot paths.
    keys: BTreeMap<*const ast::KeyPath, (ast::Shared<ast::KeyPath>, std::sync::Arc<Value>)>,
}

impl KeyPool {
    fn snapshot(&mut self, key: &ast::Shared<ast::KeyPath>) -> std::sync::Arc<Value> {
        let identity = &*key.read() as *const ast::KeyPath;
        self.keys
            .entry(identity)
            .or_insert_with(|| (key.clone(), std::sync::Arc::new(json!(key))))
            .1
            .clone()
    }
}

// Keep the larger source record and its serialization temporaries out of the
// shared lowering frame used by render/plan projections.
#[inline(never)]
pub(super) fn insert(
    compiler: &Compiler,
    field: IrNodeId,
    graph: &mut Graph,
    id: GraphNodeId,
    label: &str,
    style: &SourceNodeStyle,
    keys: &mut KeyPool,
) -> Result<(), GraphCompileError> {
    let value = object(compiler, field, graph, id, label, style, keys)?;
    graph.source_metadata_mut().objects.insert(id, value);
    Ok(())
}

fn unmeasured_label(compiler: &Compiler, field: IrNodeId) -> String {
    if let Some(value) = property_scalar_inherited(compiler, field, &["label"]) {
        return scalar_text(&value);
    }
    if let Some(primary) = compiler.arena.primary(field)
        && let IrNode::Scalar(value) = compiler.arena.node(primary)
    {
        return scalar_text(&value.value);
    }
    match compiler.arena.node(field) {
        IrNode::Field(field) => field.name.scalar_string(),
        // compileEdge promotes any link, including opaque application URLs,
        // only when neither an explicit nor primary label was supplied.
        IrNode::Edge(_) => property_text_inherited(compiler, field, &["link"]).unwrap_or_default(),
        _ => String::new(),
    }
}

pub(super) fn object(
    compiler: &Compiler,
    field: IrNodeId,
    graph: &Graph,
    id: GraphNodeId,
    _label: &str,
    style: &SourceNodeStyle,
    keys: &mut KeyPool,
) -> Result<d2_rs_core::SourceObject, GraphCompileError> {
    let mut value = source_root::object(compiler, field, false)?;
    let source_field = field;
    let IrNode::Field(field) = compiler.arena.node(field) else {
        unreachable!("graph field")
    };
    value["id"] = json!(ast::ByteString::from(format::format_key_path(
        std::slice::from_ref(&field.name)
    )));
    value["id_val"] = json!(ast::ByteString::from(field.name.scalar_bytes()));
    let node = graph.node(id).expect("compiled object");
    // TextTransform is applied by SetDimensions, after graph compilation.
    value["attributes"]["label"] = json!({"value":unmeasured_label(compiler, source_field)});
    let size = node.label_size.unwrap_or_default();
    value["attributes"]["labelDimensions"] = json!({"width":size.width,"height":size.height});
    if value["attributes"]["shape"]["value"] == "" {
        let language = value["attributes"]["language"].as_str().unwrap_or("");
        let shape = if graph.root_sequence_diagram || has_sequence_ancestor(compiler, source_field)
        {
            "rectangle"
        } else if matches!(language, "markdown" | "latex") {
            "text"
        } else if !language.is_empty() {
            "code"
        } else {
            "rectangle"
        };
        value["attributes"]["shape"] = json!({"value":shape});
    }
    if let Some(link) = &style.link {
        value["attributes"]["link"] = json!({"value":link});
    } else {
        // d2compiler.validateBoardLinks clears missing, relative and self
        // board links on the object itself, not only its target projection.
        value["attributes"].as_object_mut().unwrap().remove("link");
    }
    if let Some(bits) = style.content_aspect_ratio_bits {
        value["contentAspectRatio"] = json!(f64::from_bits(bits));
    }
    if node.shape == ShapeKind::SqlTable {
        value["sql_table"]["columns"] = if style.table_target_columns.is_empty() {
            Value::Null
        } else {
            json!(style.table_target_columns)
        };
    }
    let references = field
        .references
        .iter()
        .flatten()
        .map(|reference| {
            let reference = reference.read();
            d2_rs_core::SourceObjectReference {
                key: keys.snapshot(reference.key_path.as_ref().expect("compiled reference key")),
                key_path_index: reference.key_path_index(),
                map_key_edge_index: reference
                    .context
                    .as_ref()
                    .expect("compiled reference context")
                    .read()
                    .edge_index(),
            }
        })
        .collect::<Vec<_>>();
    Ok(d2_rs_core::SourceObject { value, references })
}

pub(super) fn edge(
    compiler: &Compiler,
    field: IrNodeId,
    graph: &Graph,
    id: d2_rs_core::EdgeId,
) -> Result<Value, GraphCompileError> {
    let base = source_root::object(compiler, field, false)?;
    let mut value = json!({"attributes":base["attributes"],"zIndex":0});
    let label = graph.edge_label(id);
    let size = label.map(|v| v.size).unwrap_or_default();
    value["attributes"]["label"] = json!({"value":unmeasured_label(compiler, field)});
    value["attributes"]["labelDimensions"] = json!({"width":size.width,"height":size.height});
    if let IrNode::Edge(edge) = compiler.arena.node(field) {
        let refs = edge.references.iter().flatten().map(|reference|json!({
            "map_key_edge_index":reference.read().context.as_ref().expect("compiled edge context").read().edge_index()
        })).collect::<Vec<_>>();
        if !refs.is_empty() {
            value["references"] = json!(refs);
        }
    }
    for (name, key, label) in [
        (
            "source-arrowhead",
            "srcArrowhead",
            &graph.edge_arrowhead_labels(id).source,
        ),
        (
            "target-arrowhead",
            "dstArrowhead",
            &graph.edge_arrowhead_labels(id).target,
        ),
    ] {
        if property_present(compiler, field, &[name]) {
            let mut attrs =
                source_root::object_at(compiler, field, false, &[name])?["attributes"].clone();
            let text = label.as_ref().map(|v| v.text.as_str()).unwrap_or("");
            let size = label.as_ref().map(|v| v.size).unwrap_or_default();
            attrs["label"] = json!({"value":text});
            attrs["labelDimensions"] = json!({"width":size.width,"height":size.height});
            value[key] = attrs;
        }
    }
    Ok(value)
}
