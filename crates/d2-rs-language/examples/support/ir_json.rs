// SPDX-License-Identifier: MPL-2.0
//! Complete encoding/json projection shared by language scoreboard runners.
use d2_rs_core::{ast, ir};
use serde_json::{Value, json};

fn context(value: &Option<ast::Shared<ir::RefContext>>) -> Value {
    value.as_ref().map_or(Value::Null, |value| {
        let value = value.read();
        json!({"edge":value.edge, "key":value.key})
    })
}

// The upstream goldens use encoding/json on d2ir nodes, including complete
// lexical references, nil slices and concrete scalar objects. A summary or
// formatted AST is not a replacement for that contract.
pub fn ir_json(arena: &ir::Arena, id: ir::NodeId) -> Value {
    if arena.is_nil_scalar(id) {
        return Value::Null;
    }
    match arena.node(id) {
        ir::Node::Map(map) => json!({
            "fields":map.fields.as_ref().map(|items| items.iter().map(|id| ir_json(arena,*id)).collect::<Vec<_>>()),
            "edges":map.edges.as_ref().map(|items| items.iter().map(|id| ir_json(arena,*id)).collect::<Vec<_>>()),
        }),
        ir::Node::Scalar(value) => json!({"value":value.value.unbox()}),
        ir::Node::Array(array) => {
            json!({"values":array.values.as_ref().map(|items| items.iter().map(|id| ir_json(arena,*id)).collect::<Vec<_>>())})
        }
        ir::Node::Field(field) => {
            let mut value = json!({"name":field.name.unbox()});
            if let Some(id) = field.primary {
                value["primary"] = ir_json(arena, id);
            }
            if let Some(id) = field.composite {
                value["composite"] = ir_json(arena, id);
            }
            if let Some(refs) = field.references.as_ref().filter(|refs| !refs.is_empty()) {
                value["references"] = json!(
                    refs.iter()
                        .map(|reference| {
                            let reference = reference.read();
                            json!({"string":reference.string,"key_path":reference.key_path,
                        "context":context(&reference.context),"due_to_glob":reference.due_to_glob,
                        "due_to_lazy_glob":reference.due_to_lazy_glob})
                        })
                        .collect::<Vec<_>>()
                );
            }
            value
        }
        ir::Node::Edge(edge) => {
            let mut value = json!({"edge_id":edge.id});
            if let Some(id) = edge.primary {
                value["primary"] = ir_json(arena, id);
            }
            if let Some(id) = edge.map {
                value["map"] = ir_json(arena, id);
            }
            if let Some(refs) = edge.references.as_ref().filter(|refs| !refs.is_empty()) {
                value["references"] = json!(refs.iter().map(|reference| {
                    let reference = reference.read();
                    json!({"context":context(&reference.context),"due_to_glob":reference.due_to_glob,
                        "due_to_lazy_glob":reference.due_to_lazy_glob})
                }).collect::<Vec<_>>());
            }
            value
        }
    }
}
