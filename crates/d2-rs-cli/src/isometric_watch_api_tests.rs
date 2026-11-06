// SPDX-License-Identifier: MPL-2.0
//! Input-only transport for original raw-target watch preview calls.
use super::*;
use serde_json::{Value, json};

fn expand(nodes: &[Value], index: usize, ancestors: &mut Vec<usize>) -> Option<Value> {
    // Owned native targets cannot represent Go pointer cycles.
    // Keep these inputs unsupported rather than turning that into error parity.
    if ancestors.contains(&index) {
        return None;
    }
    ancestors.push(index);
    let mut value = nodes[index]["diagram"].clone();
    for field in ["layers", "scenarios", "steps"] {
        let children = nodes[index][field].as_array().unwrap();
        if !children.is_empty() {
            value[field] = children
                .iter()
                .map(|child| expand(nodes, child.as_u64().unwrap() as usize, ancestors))
                .collect::<Option<Vec<_>>>()?
                .into();
        }
    }
    ancestors.pop();
    Some(value)
}

fn paths(value: &Value, key: String, parts: Vec<String>, out: &mut BTreeMap<String, String>) {
    let path = navigation_path(&parts);
    out.insert(key.clone(), path);
    for field in ["layers", "scenarios", "steps"] {
        if let Some(children) = value[field].as_array() {
            for child in children {
                let name = child["name"].as_str().unwrap();
                let mut next = parts.clone();
                next.extend([field.into(), name.into()]);
                paths(child, format!("{key}.{field}.{name}"), next, out);
            }
        }
    }
}

fn observe(input: &Value) -> Value {
    let tree = &input["tree"];
    let nodes = tree["nodes"].as_array().unwrap();
    let Some(root) = expand(
        nodes,
        tree["root"].as_u64().unwrap() as usize,
        &mut Vec::new(),
    ) else {
        return json!({"unsupported": "Go pointer-cycle target cannot enter the owned native target API"});
    };
    let selected = expand(
        nodes,
        tree["selected"].as_u64().unwrap() as usize,
        &mut Vec::new(),
    )
    .unwrap();
    let mut links = BTreeMap::new();
    paths(
        &root,
        input["source_root_id"].as_str().unwrap().into(),
        Vec::new(),
        &mut links,
    );
    let opts = &input["render"];
    let result = render_target_preview(
        &selected,
        &links,
        input["input_path"].as_str().unwrap(),
        opts["ThemeID"].as_i64(),
        opts["Pad"].as_f64(),
        opts["Scale"].as_f64(),
        opts["Salt"].as_str(),
        opts["Center"].as_bool(),
        opts["NoXMLTag"].as_bool().unwrap_or(false),
        opts["OmitVersion"].as_bool().unwrap_or(false),
    );
    let (svg, error) = match result {
        Ok(svg) => (String::from_utf8(svg).unwrap(), String::new()),
        Err(error) => (String::new(), error),
    };
    json!({"svg": svg, "error": error, "tree_after": tree, "render_after": opts})
}

#[test]
#[ignore = "original raw-target watch preview replay"]
fn replay() {
    let request = std::env::var_os("D2RS_WATCH_REQUEST").unwrap();
    let response = std::env::var_os("D2RS_WATCH_RESPONSE").unwrap();
    let inputs: Vec<Value> = serde_json::from_slice(&std::fs::read(request).unwrap()).unwrap();
    let results: Vec<_> = inputs.iter().map(observe).collect();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(response)
        .unwrap();
    serde_json::to_writer(&mut file, &results).unwrap();
}
