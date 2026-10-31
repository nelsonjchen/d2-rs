// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{
    ast::{self, Shared},
    ir::*,
};
use d2_rs_language::{format, ir, parse};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize)]
pub struct Request {
    input: String,
    path: String,
    files: Option<std::collections::HashMap<String, String>>,
    files_hex: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    utf16: bool,
}
fn name(arena: &Arena, node: NodeId) -> Option<String> {
    if ir::ida(arena, node).iter().flatten().any(Option::is_none) {
        return None;
    }
    let path: Vec<_> = ir::ida(arena, node)
        .into_iter()
        .flatten()
        .map(|p| ast::raw_string_box(ast::ScalarBox::from_child(p).scalar_bytes(), true))
        .collect();
    Some(String::from_utf8(format::format_key_path(&path)).unwrap())
}
fn visit(arena: &Arena, node: NodeId, rows: &mut Vec<Value>) {
    if arena.is_nil_scalar(node) {
        rows.push(json!({"path":null,"kind":"scalar","nil":true}));
        return;
    }
    let mut row = json!({"path":name(arena,node)});
    let children = match arena.node(node) {
        Node::Map(m) => {
            row["kind"] = json!("map");
            row["version"] = json!(m.structure_version);
            row["globs"]=json!(m.globs.iter().flat_map(ast::SharedSlice::iter).map(|id|{let g=arena.glob(id.read().unwrap());let c=g.context.as_ref().unwrap().read();let mut fields:Vec<_>=g.applied_fields.as_ref().map(|s|s.read().clone()).into_iter().flatten().map(|s|ast::ByteString::from(s.clone()).to_text().into_owned()).collect();fields.sort();let mut edges:Vec<_>=g.applied_edges.as_ref().map(|s|s.read().clone()).into_iter().flatten().map(|s|ast::ByteString::from(s).to_text().into_owned()).collect();edges.sort();json!({"edges":edges,"key":c.key.as_ref().map(|k|String::from_utf8(format::format(&ast::Child::Key(k.clone()))).unwrap()),"scope":c.scope_map.and_then(|scope|name(arena,scope)),"fields":fields})}).collect::<Vec<_>>());
            m.fields
                .iter()
                .flatten()
                .chain(m.edges.iter().flatten())
                .copied()
                .collect::<Vec<_>>()
        }
        Node::Field(f) => {
            row["kind"] = json!("field");
            row["suspended"] = json!(f.suspended);
            row["refs"]=json!(f.references.iter().flatten().map(|r|{let r=r.read();let c=r.context.as_ref().unwrap().read();json!({"key":c.key.as_ref().map(|k|String::from_utf8(format::format(&ast::Child::Key(k.clone()))).unwrap()),"string":r.string,"glob":r.due_to_glob,"lazy":r.due_to_lazy_glob,"scope":c.scope_map.and_then(|scope|name(arena,scope))})}).collect::<Vec<_>>());
            f.primary.into_iter().chain(f.composite).collect()
        }
        Node::Array(a) => {
            row["kind"] = json!("array");
            a.values.clone().unwrap_or_default()
        }
        Node::Scalar(_) => {
            row["kind"] = json!("scalar");
            Vec::new()
        }
        Node::Edge(e) => {
            row["kind"] = json!("edge");
            row["id"] = json!(e.id);
            row["suspended"] = json!(e.suspended);
            row["refs"]=json!(e.references.iter().flatten().map(|r|{let r=r.read();let c=r.context.as_ref().unwrap().read();json!({"key":c.key.as_ref().map(|k|String::from_utf8(format::format(&ast::Child::Key(k.clone()))).unwrap()),"edge":c.edge,"glob":r.due_to_glob,"lazy":r.due_to_lazy_glob,"scope":c.scope_map.and_then(|scope|name(arena,scope))})}).collect::<Vec<_>>());
            e.primary.into_iter().chain(e.map).collect()
        }
    };
    rows.push(row);
    for child in children {
        visit(arena, child, rows)
    }
}
pub fn evaluate(r: Request) -> Value {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| evaluate_inner(r)))
        .unwrap_or_else(|_| json!({"panic":true}))
}
fn evaluate_inner(r: Request) -> Value {
    let parsed = parse(
        r.path,
        r.input.as_bytes(),
        Some(d2_rs_language::ParseOptions {
            utf16_positions: r.utf16,
            errors: None,
        }),
    );
    if let Some(e) = parsed.error {
        return json!({"parse_error":e.to_string()});
    }
    let imports = r.files.is_some() || r.files_hex.is_some();
    let mut options = d2_rs_language::compiler::CompileOptions {
        utf16_positions: r.utf16,
        ..Default::default()
    };
    if imports {
        let mut files = r
            .files
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (k, v.into_bytes()))
            .collect::<std::collections::HashMap<_, _>>();
        for (path, hex) in r.files_hex.unwrap_or_default() {
            files.insert(
                path,
                (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                    .collect(),
            );
        }
        options.import_opener = Some(Box::new(move |path| {
            files
                .get(path)
                .map(|bytes| {
                    Box::new(std::io::Cursor::new(bytes.clone()))
                        as Box<dyn d2_rs_language::stream::ByteReader>
                })
                .ok_or_else(|| format!("open {path}: file does not exist"))
        }));
    }
    let result =
        d2_rs_language::compiler::compile_ir_with_options(Shared::new(parsed.map), options);
    let (c, root) = match result {
        Ok(result) => result,
        Err(e) => return json!({"untranslated":e.feature,"range":e.range}),
    };
    let mut nodes = Vec::new();
    visit(&c.arena, root, &mut nodes);
    let projected = if imports {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| json!(c.arena.ast(root))))
            .unwrap_or_else(|_| json!({"panic":true}))
    } else {
        json!(c.arena.ast(root))
    };
    let mut output = json!({"ast":projected,"errors":c.errors,"nodes":nodes});
    if imports {
        let mut parsed = c.import_state.parsed.keys().collect::<Vec<_>>();
        parsed.sort();
        let mut templates = c.import_templates.keys().collect::<Vec<_>>();
        templates.sort();
        let mut seen = c.seen_imports.iter().collect::<Vec<_>>();
        seen.sort();
        output["parsed"] = json!(parsed);
        output["templates"] = json!(templates);
        output["seen"] = json!(seen);
        output["imports"] = if c.imports.is_empty() {
            Value::Null
        } else {
            json!(c.imports)
        };
    }
    #[cfg(feature = "compiler-trace")]
    let output = {
        let mut output = output;
        output["trace"] = json!(c.key_trace);
        output
    };
    output
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    println!(
        "{}",
        json!(requests.into_iter().map(evaluate).collect::<Vec<_>>())
    );
}
