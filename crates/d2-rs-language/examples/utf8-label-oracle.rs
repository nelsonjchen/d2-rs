// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::ast::Shared;
use d2_rs_language::{compiler, parse};
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn main() {
    let mut rows: Vec<serde_json::Value> = serde_json::from_reader(std::io::stdin()).unwrap();
    for row in &mut rows {
        let input = row["input"].as_str().unwrap();
        let parsed = parse("source.d2", input.as_bytes(), None);
        if let Some(error) = parsed.error {
            row["compile_error"] = serde_json::json!(error.to_string());
            continue;
        }
        let (compiled, root) = match compiler::compile_ir(Shared::new(parsed.map)) {
            Ok(value) => value,
            Err(error) => {
                row["compile_error"] = serde_json::json!(error.to_string());
                continue;
            }
        };
        let graph = match compiler::lower_graph(&compiled, root) {
            Ok(graph) => graph,
            Err(error) => {
                row["compile_error"] = serde_json::json!(error.to_string());
                continue;
            }
        };
        row["objects"]=serde_json::json!(graph.nodes().map(|(id,node)| {
            let label=graph.node_label_bytes.get(&id).unwrap();
            serde_json::json!({"id":node.external_id,"after_hex":hex(label),"valid_utf8":std::str::from_utf8(label).is_ok()})
        }).collect::<Vec<_>>());
    }
    println!("{}", serde_json::to_string_pretty(&rows).unwrap());
}
