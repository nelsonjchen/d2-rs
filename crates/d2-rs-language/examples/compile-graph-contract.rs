// SPDX-License-Identifier: MPL-2.0
//! Development observer for public source-to-graph acceptance and diagnostics.
use d2_rs_core::{Size, ast::Shared};
use d2_rs_language::{ParseOptions, compiler, parse};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
#[derive(Deserialize)]
struct Request {
    hex: String,
    path: String,
    #[serde(default)]
    utf16: bool,
    #[serde(default)]
    measured: bool,
    #[serde(default)]
    files_hex: HashMap<String, String>,
}
fn decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
struct Unmeasured;
impl compiler::GraphMeasurer for Unmeasured {
    fn measure(&self, _: &str) -> Size {
        Size::default()
    }
}
fn evaluate(r: Request) -> Value {
    let parsed = parse(
        r.path,
        &decode(&r.hex),
        Some(ParseOptions {
            utf16_positions: r.utf16,
            errors: None,
        }),
    );
    if let Some(error) = parsed.error {
        return json!({"error":error.to_string(),"panic":false});
    }
    let files = r
        .files_hex
        .into_iter()
        .map(|(p, s)| (p, decode(&s)))
        .collect::<HashMap<_, _>>();
    let options = compiler::CompileOptions {
        utf16_positions: r.utf16,
        import_opener: Some(Box::new(move |path| {
            files
                .get(path)
                .map(|bytes| {
                    Box::new(std::io::Cursor::new(bytes.clone()))
                        as Box<dyn d2_rs_language::stream::ByteReader>
                })
                .ok_or_else(|| format!("open {path}: file does not exist"))
        })),
    };
    let (c, root) = match compiler::compile_ir_with_options(Shared::new(parsed.map), options) {
        Ok(v) => v,
        Err(error) => return json!({"error":error.to_string(),"panic":false,"untranslated":true}),
    };
    let (graph, labels, node_styles) =
        match compiler::lower_graph_with_measurer_and_labels(&c, root, &Unmeasured) {
            Ok(graph) => graph,
            Err(error) => return json!({"error":error.to_string(),"panic":false}),
        };
    let measured = if r.measured {
        Some(match compiler::lower_graph(&c, root) {
            Ok(graph) => {
                json!({"error":null,"node_label_bytes":graph.node_label_bytes,"graph":graph})
            }
            Err(error) => json!({"error":error.to_string()}),
        })
    } else {
        None
    };
    match compiler::extract_config_checked(&c, root) {
        Ok(config) => {
            json!({"error":null,"panic":false,"ir_fields":match c.arena.node(root){d2_rs_core::ir::Node::Map(map)=>map.fields.clone().unwrap_or_default().into_iter().map(|id|match c.arena.node(id){d2_rs_core::ir::Node::Field(f)=>f.name.scalar_string(),_=>String::new()}).collect::<Vec<_>>(),_=>Vec::new()},"measured":measured,"node_label_bytes":graph.node_label_bytes,"node_order":graph.node_order_override,"hierarchy_order":graph.hierarchy_children_order.iter().collect::<Vec<_>>(),"root_style":graph.root_style,"graph":graph,"labels":labels,"node_styles":node_styles,"config":config})
        }
        Err(errors) => {
            json!({"error":errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"),"panic":false})
        }
    }
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    let records = requests
        .into_iter()
        .map(|r| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| evaluate(r)))
                .unwrap_or_else(|_| json!({"panic":true,"error":null}))
        })
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string(&records).unwrap());
}
