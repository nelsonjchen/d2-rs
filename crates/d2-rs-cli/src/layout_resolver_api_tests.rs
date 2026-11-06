// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of the original resolver calls on flat measured graphs.
use super::LayoutResolver;
use d2_rs_core::{Edge, EdgeArrows, Graph, LayoutEngineOptions, LayoutOptions, Node, NodeId, Size};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Barrier},
};

fn options(flags: &Value) -> (LayoutEngineOptions, LayoutOptions) {
    let mut settings = LayoutEngineOptions::default();
    let mut options = LayoutOptions::default();
    for (name, value) in flags.as_object().unwrap() {
        if name == "tala-seeds" {
            options.seeds = serde_json::from_value(value.clone()).unwrap();
        } else {
            crate::command_line::set_engine_option(
                &mut settings,
                name,
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string()),
            );
        }
    }
    (settings, options)
}

fn measured(input: &Value) -> Graph {
    let mut graph = Graph::default();
    let mut ids = BTreeMap::new();
    let objects = input["objects"].as_array().unwrap();
    assert!(
        input.get("layers").is_none()
            && input.get("scenarios").is_none()
            && input.get("steps").is_none()
    );
    for object in objects {
        // Fail closed outside the captured flat, ordinary-node API domain.
        assert!(object.get("ChildrenArray").is_none());
        assert!(matches!(
            object["attributes"]["shape"]["value"].as_str(),
            Some("" | "rectangle")
        ));
        let id = object["AbsID"].as_str().unwrap();
        let mut node = Node::new(
            id,
            Size {
                width: object["box"]["Width"].as_f64().unwrap(),
                height: object["box"]["Height"].as_f64().unwrap(),
            },
        );
        let dims = &object["attributes"]["labelDimensions"];
        // d2graph.Object.Text defaults ordinary objects to FONT_SIZE_M;
        // adapter.go carries that scalar even for an empty label.
        node.font_size = Some(
            object["attributes"]["style"]["fontSize"]["value"]
                .as_str()
                .map(|v| v.parse().unwrap())
                .unwrap_or(16),
        );
        if object["attributes"]["label"]["value"].as_str().unwrap() != "" {
            node.label_size = Some(Size {
                width: dims["width"].as_f64().unwrap(),
                height: dims["height"].as_f64().unwrap(),
            });
        }
        ids.insert(id, graph.add_node(node));
    }
    if let Some(edges) = input["edges"].as_array() {
        for edge in edges {
            assert_eq!(edge["attributes"]["label"]["value"], "");
            let id = graph.add_edge(Edge {
                source: ids[edge["Src"].as_str().unwrap()],
                target: ids[edge["Dst"].as_str().unwrap()],
            });
            graph.set_edge_arrows(
                id,
                EdgeArrows {
                    source: edge["src_arrow"].as_bool().unwrap(),
                    target: edge["dst_arrow"].as_bool().unwrap(),
                },
            );
        }
    }
    if let Some(seeds) = input.get("data").and_then(|d| d.get("tala-seeds")) {
        graph.tala_seed_data =
            Some(serde_json::from_value(seeds.clone()).map_err(|e| e.to_string()));
    }
    graph
}

fn invoke(resolver: &LayoutResolver, request: &Value) -> Value {
    let layout = resolver.resolve(request["name"].as_str().unwrap()).unwrap();
    let mut graph = measured(&request["input"]);
    let mut output = request["input"].clone();
    let result = match layout.layout(&mut graph) {
        Ok(result) => result,
        Err(error) => return json!({"error":error.to_string(),"graph":output}),
    };
    assert!(result.removed_edges.is_empty());
    for (i, object) in output["objects"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        let id = NodeId(i as u32);
        let rect = result.boxes[&id];
        object["box"] =
            json!({"Width":rect.size.width,"Height":rect.size.height,"TopLeft":rect.origin});
        if let Some(label) = result.node_labels.get(&id) {
            if label.position.d2_name() != "" {
                object["labelPosition"] = json!(label.position.d2_name());
            }
            if label.font_size_committed {
                if let Some(size) = label.font_size {
                    object["attributes"]["style"]["fontSize"] = json!({"value":size.to_string()});
                }
            }
        }
        // Same ordinary-node fallback as the serialized graph bridge:
        // DefaultLabelPositions materializes an inside position for text.
        if object.get("labelPosition").is_none()
            && object["attributes"]["label"]["value"].as_str().unwrap() != ""
        {
            object["labelPosition"] = json!("INSIDE_MIDDLE_CENTER");
        }
    }
    if let Some(edges) = output["edges"].as_array_mut() {
        for (i, edge) in edges.iter_mut().enumerate() {
            let id = d2_rs_core::EdgeId(i as u32);
            edge["route"] = serde_json::to_value(&result.routes[&id]).unwrap();
            edge["isCurve"] = json!(result.route_curves.get(&id).copied().unwrap_or(false));
        }
    }
    json!({"error":"","graph":output})
}

#[test]
#[ignore = "explicit input-only original Go resolver replay"]
fn replay() {
    let requests: Vec<Value> = serde_json::from_slice(
        &std::fs::read(std::env::var("D2RS_RESOLVER_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let mut resolvers = BTreeMap::new();
    let mut caller_options = BTreeMap::new();
    let mut results = vec![Value::Null; requests.len()];
    let mut concurrent = Vec::new();
    for (i, request) in requests.iter().enumerate() {
        let id = request["resolver"].as_u64().unwrap();
        match request["op"].as_str().unwrap() {
            "create" => {
                let (settings, options) = options(&request["flags"]);
                let resolver = LayoutResolver::new(&settings, &options);
                caller_options.insert(id, (settings, options));
                resolvers.insert(id, Arc::new(resolver));
                results[i] = json!({"error":""});
            }
            "resolve" => {
                // Apply the original flag/slice mutations to caller-owned
                // storage after construction, leaving captured options alone.
                *caller_options.get_mut(&id).unwrap() = options(&request["flags"]);
                let error = resolvers[&id]
                    .resolve(request["name"].as_str().unwrap())
                    .err()
                    .unwrap_or_default();
                results[i] = json!({"error":error});
            }
            "layout"
                if request["test"]
                    .as_str()
                    .unwrap()
                    .starts_with("TestConcurrentResolvedLayouts/") =>
            {
                concurrent.push((i, id, request.clone()))
            }
            "layout" => results[i] = invoke(&resolvers[&id], request),
            _ => panic!("unknown resolver event"),
        }
    }
    // Original calls overlap in batches of eight, sharing one resolver. Both
    // resolve and layout run after a barrier on real native worker threads.
    for batch in concurrent.chunks(8) {
        assert_eq!(batch.len(), 8);
        assert!(batch.iter().all(|(_, id, _)| *id == batch[0].1));
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = batch
            .iter()
            .map(|(i, id, request)| {
                let (i, request, resolver, barrier) = (
                    *i,
                    request.clone(),
                    Arc::clone(&resolvers[id]),
                    Arc::clone(&barrier),
                );
                std::thread::spawn(move || {
                    barrier.wait();
                    (i, invoke(&resolver, &request))
                })
            })
            .collect();
        for handle in handles {
            let (i, result) = handle.join().unwrap();
            results[i] = result;
        }
    }
    std::fs::write(
        std::env::var("D2RS_RESOLVER_RESPONSE").unwrap(),
        serde_json::to_vec(&results).unwrap(),
    )
    .unwrap();
}
