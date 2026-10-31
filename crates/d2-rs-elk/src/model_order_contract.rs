// SPDX-License-Identifier: MPL-2.0
//! Original Go geometry contracts; complete raw input/output is retained.
//! Contracts distinguish modeled geometry checks from complete raw JSON checks.
use crate::{ElkGraph, ElkOptions, layout_elk_graph_with_layered_pipeline};
use serde_json::Value;

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn number(source: &Value, native: &Value, key: &str, path: &str) {
    let a = source[key].as_f64().unwrap();
    let b = native[key].as_f64().unwrap();
    assert_eq!(a.to_bits(), b.to_bits(), "{path}.{key}: {a} vs {b}");
}

fn boxes(source: &Value, native: &Value, key: &str, path: &str) {
    let a = array(source, key);
    let b = array(native, key);
    assert_eq!(a.len(), b.len(), "{path}.{key} length");
    for (index, (a, b)) in a.iter().zip(b).enumerate() {
        let path = format!("{path}.{key}[{index}]");
        for identity in ["id", "text"] {
            assert_eq!(a.get(identity), b.get(identity), "{path}.{identity}");
        }
        for field in ["x", "y", "width", "height"] {
            number(a, b, field, &path);
        }
        for child in ["children", "ports", "labels"] {
            boxes(a, b, child, &path);
        }
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_nested_inverted_model_order_boxes_and_routes() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_nested_model_order.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 32);
    compare_layout_rows(rows);
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_nested_cross_hierarchy_sections_use_the_edge_container_frame() {
    // Reduced from regression/dagre-disconnect and stable/dagre_spacing.
    // The outer incoming edge places the common container away from zero;
    // its internal hierarchy edge must not include that origin twice.
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_nested_edge_container.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 4);
    compare_layout_rows(rows);
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_cycle_label_geometry_all_directions() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_cycle_geometry.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 12);
    compare_layout_rows(rows);
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_sql_geometry_all_directions() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_fixed_sql_ports.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 8);
    compare_layout_rows(rows);
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_mixed_loop_geometry_all_directions() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_mixed_loop_geometry.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 4);
    compare_layout_rows(rows);
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_broader_loop_geometry_all_directions() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_broader_loop_geometry.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 12);
    compare_layout_rows(rows);
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_greedy_cycle_seeds_and_direction_priority() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_greedy_cycle_geometry.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 10);
    compare_layout_rows(rows);
}

#[test]
fn original_go_self_loop_hidden_port_membership_and_restore_identity() {
    use crate::layered::{
        LayeredArena, hide_self_loops_for_ordering, restore_pure_loop_port_areas_for_graph,
        restore_self_loop_edges_after_routing,
    };
    use serde_json::json;
    for constraints in [
        "FREE",
        "FIXED_SIDE",
        "FIXED_ORDER",
        "FIXED_RATIO",
        "FIXED_POS",
    ] {
        let ports = [
            "WEST", "NORTH", "NORTH", "EAST", "EAST", "SOUTH", "SOUTH", "WEST",
        ]
        .iter()
        .enumerate()
        .map(|(i, side)| {
            json!({"id": format!("b.{i}"), "x": 0, "y": 0,
                "width": 0, "height": 0, "layoutOptions": {"elk.port.side": side}})
        })
        .collect::<Vec<_>>();
        let mut edges = vec![json!({"id": "regular", "sources": ["a.p"], "targets": ["b.0"]})];
        for (i, (source, target)) in [(0, 1), (1, 2), (3, 4), (4, 5)].into_iter().enumerate() {
            edges.push(
                json!({"id": format!("loop{i}"), "sources": [format!("b.{source}")],
                "targets": [format!("b.{target}")]}),
            );
        }
        let graph: ElkGraph = serde_json::from_value(json!({"id": "root", "layoutOptions": {},
            "children": [{"id":"a","width":50,"height":50,"ports":[{"id":"a.p","x":0,"y":0,
                "width":0,"height":0,"layoutOptions":{"elk.port.side":"EAST"}}]},
                {"id":"b","width":50,"height":50,"ports":ports,
                    "layoutOptions":{"elk.portConstraints": constraints}}], "edges":edges}))
        .unwrap();
        let (mut arena, root) = LayeredArena::from_elk_graph(&graph).unwrap();
        let original = arena.nodes[1].ports.clone();
        hide_self_loops_for_ordering(&mut arena);
        if matches!(constraints, "FREE" | "FIXED_SIDE") {
            assert_eq!(
                arena.nodes[1].ports,
                vec![original[0], original[6], original[7]],
                "{constraints}"
            );
        } else {
            assert_eq!(arena.nodes[1].ports, original, "{constraints}");
        }
        for port in &original {
            let port = &arena.ports[*port as usize];
            assert!(
                port.incoming_edges
                    .iter()
                    .chain(&port.outgoing_edges)
                    .all(|id| arena.edges[*id as usize].source != arena.edges[*id as usize].target)
            );
        }
        restore_pure_loop_port_areas_for_graph(&mut arena, root);
        let mut restored = arena.nodes[1].ports.clone();
        restored.sort_unstable();
        let mut expected = original.clone();
        expected.sort_unstable();
        assert_eq!(restored, expected, "restored identity {constraints}");
        // Source PortRestorer restores nodes/ports before P4; edge adjacency
        // stays detached until SelfLoopPostprocessor after P5.
        for port in &original {
            let port = &arena.ports[*port as usize];
            assert!(
                port.incoming_edges
                    .iter()
                    .chain(&port.outgoing_edges)
                    .all(|id| {
                        let edge = &arena.edges[*id as usize];
                        edge.source != edge.target
                    }),
                "loop adjacency restored prematurely: {constraints}"
            );
        }
        restore_self_loop_edges_after_routing(&mut arena, root);
        for (id, edge) in arena.edges.iter().enumerate() {
            if edge.source != edge.target {
                continue;
            }
            let source = edge.source_port.unwrap();
            let target = edge.target_port.unwrap();
            assert_eq!(
                arena.ports[source as usize]
                    .outgoing_edges
                    .iter()
                    .filter(|e| **e == id as u32)
                    .count(),
                1
            );
            assert_eq!(
                arena.ports[target as usize]
                    .incoming_edges
                    .iter()
                    .filter(|e| **e == id as u32)
                    .count(),
                1
            );
        }
    }
}

fn compare_layout_rows(rows: Vec<Value>) {
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        let actual = serde_json::to_value(graph).unwrap();
        let expected = &row["expected"];
        for field in ["width", "height"] {
            number(expected, &actual, field, name);
        }
        boxes(expected, &actual, "children", name);
        let source_edges = array(expected, "edges");
        let native_edges = array(&actual, "edges");
        assert_eq!(source_edges.len(), native_edges.len(), "{name}: edges");
        for (index, (a, b)) in source_edges.iter().zip(native_edges).enumerate() {
            let path = format!("{name}.edges[{index}]");
            for key in ["id", "sources", "targets", "container"] {
                assert_eq!(a.get(key), b.get(key), "{path}.{key}");
            }
            boxes(a, b, "labels", &path);
            let sections_a = array(a, "sections");
            let sections_b = array(b, "sections");
            assert_eq!(sections_a.len(), sections_b.len(), "{path}: sections");
            for (index, (a, b)) in sections_a.iter().zip(sections_b).enumerate() {
                let path = format!("{path}.sections[{index}]");
                for key in ["id", "incomingShape", "outgoingShape"] {
                    assert_eq!(a.get(key), b.get(key), "{path}.{key}");
                }
                for key in ["startPoint", "endPoint"] {
                    for axis in ["x", "y"] {
                        number(&a[key], &b[key], axis, &format!("{path}.{key}"));
                    }
                }
                let bends_a = array(a, "bendPoints");
                let bends_b = array(b, "bendPoints");
                assert_eq!(bends_a.len(), bends_b.len(), "{path}: bend count");
                for (index, (a, b)) in bends_a.iter().zip(bends_b).enumerate() {
                    for axis in ["x", "y"] {
                        number(a, b, axis, &format!("{path}.bendPoints[{index}]"));
                    }
                }
            }
        }
    }
}

// Node/edge placement for these controls is qualified separately. Fixed port
// coordinates have their own source contract; complete raw outputs remain in
// the fixture; full geometry is checked by the separate contract above.
#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_fixed_sql_port_coordinates_all_directions() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_fixed_sql_ports.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 8);
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        let actual = serde_json::to_value(graph).unwrap();
        let source_nodes = array(&row["expected"], "children");
        let native_nodes = array(&actual, "children");
        assert_eq!(source_nodes.len(), native_nodes.len(), "{name}: nodes");
        let mut ports = 0;
        for (source, native) in source_nodes.iter().zip(native_nodes) {
            assert_eq!(source["id"], native["id"], "{name}: node order");
            ports += array(source, "ports").len();
            boxes(source, native, "ports", name);
        }
        assert!(ports > 0, "{name}: nonempty port contract");
    }
}

#[test]
fn original_go_vertical_center_label_dummy_aggregation() {
    // Same internal label dimensions as Go's
    // TestInsertLabelDummiesMovesOnlyCenterLabels: (20,7),(30,9), spacing3.
    // This wire boundary transposes physical dimensions for DOWN.
    let wire: ElkGraph = serde_json::from_value(serde_json::json!({
        "id":"root",
        "layoutOptions":{"elk.direction":"DOWN","elk.spacing.edgeLabel":2,"elk.spacing.labelLabel":3},
        "children":[{"id":"a","width":40,"height":30},{"id":"b","width":40,"height":30}],
        "edges":[{"id":"e","sources":["a"],"targets":["b"],"labels":[
            {"text":"a","width":7,"height":20},
            {"text":"tail","width":1,"height":1,"layoutOptions":{"elk.edgeLabels.placement":"TAIL"}},
            {"text":"b","width":9,"height":30}
        ]}]
    })).unwrap();
    let (mut arena, root) = crate::LayeredArena::from_elk_graph(&wire).unwrap();
    arena
        .insert_center_edge_label_dummies(root, &wire.edges, true)
        .unwrap();
    assert_eq!(arena.edge_label_dummies.len(), 1);
    let dummy = &arena.edge_label_dummies[0];
    assert_eq!(dummy.label_indices, [0, 2]);
    let node = &arena.nodes[dummy.node as usize];
    assert_eq!(
        (node.width.to_bits(), node.height.to_bits()),
        (53.0_f64.to_bits(), 12.0_f64.to_bits())
    );
    assert!(
        node.ports
            .iter()
            .all(|id| arena.ports[*id as usize].y.to_bits() == 0.0_f64.to_bits())
    );
}

// Complete JSON contract: field presence, identity/order and signed f64 bits.
fn assert_raw_graph(source: &Value, native: &Value, path: &str) {
    match (source, native) {
        (Value::Number(a), Value::Number(b)) if path.ends_with(".id") => {
            assert_eq!(a, b, "{path}: exact numeric ID representation");
        }
        (Value::Number(a), Value::Number(b)) => assert_eq!(
            a.as_f64().unwrap().to_bits(),
            b.as_f64().unwrap().to_bits(),
            "{path}"
        ),
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(
                a.keys().collect::<std::collections::BTreeSet<_>>(),
                b.keys().collect::<std::collections::BTreeSet<_>>(),
                "{path} keys"
            );
            for (key, value) in a {
                assert_raw_graph(value, &b[key], &format!("{path}.{key}"));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path} length");
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                assert_raw_graph(a, b, &format!("{path}[{index}]"));
            }
        }
        _ => assert_eq!(source, native, "{path}"),
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_root_coordinates_and_empty_graph_raw_output() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_root_coordinates.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 56);
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
        let before = serde_json::to_value(&graph).unwrap();
        for key in ["x", "y"] {
            match row["input"].get(key).filter(|value| !value.is_null()) {
                Some(value) => assert_raw_graph(value, &before[key], name),
                None => assert!(before.get(key).is_none(), "{name}.{key} before layout"),
            }
        }
        layout_elk_graph_with_layered_pipeline(
            &mut graph,
            ElkOptions {
                node_spacing: 20.0,
                edge_node_spacing: 10.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();
        assert_raw_graph(
            &row["expected"],
            &serde_json::to_value(&graph).unwrap(),
            name,
        );
    }
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_existing_loop_sql_and_greedy_complete_raw_output() {
    let fixtures = [
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_fixed_sql_ports.json"),
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_mixed_loop_geometry.json"),
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_broader_loop_geometry.json"),
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_greedy_cycle_geometry.json"),
    ];
    let mut count = 0;
    for fixture in fixtures {
        for row in serde_json::from_str::<Vec<Value>>(fixture).unwrap() {
            let name = row["name"].as_str().unwrap();
            let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
            layout_elk_graph_with_layered_pipeline(
                &mut graph,
                ElkOptions {
                    node_spacing: 20.0,
                    edge_node_spacing: 10.0,
                    ..ElkOptions::default()
                },
            )
            .unwrap();
            assert_raw_graph(
                &row["expected"],
                &serde_json::to_value(&graph).unwrap(),
                name,
            );
            count += 1;
        }
    }
    assert_eq!(count, 34);
}

#[test]
fn original_go_included_hierarchy_direction_import_contract() {
    use crate::layered::{LayerDirection, LayeredArena};
    use serde_json::json;
    let directions = [
        ("DOWN", LayerDirection::Down),
        ("RIGHT", LayerDirection::Right),
        ("UP", LayerDirection::Up),
        ("LEFT", LayerDirection::Left),
    ];
    for (root_name, root_direction) in directions {
        for (child_name, child_direction) in directions {
            for policy in ["INHERIT", "INCLUDE_CHILDREN", "SEPARATE_CHILDREN"] {
                let graph: ElkGraph = serde_json::from_value(json!({
                    "id":"root", "layoutOptions":{
                        "elk.direction":root_name, "elk.hierarchyHandling":"INCLUDE_CHILDREN"
                    }, "children":[{"id":"compound", "width":0, "height":0,
                        "layoutOptions":{"elk.direction":child_name,"elk.hierarchyHandling":policy},
                        "children":[{"id":"inner", "width":0, "height":0,
                            "layoutOptions":{"elk.direction":child_name},
                            "children":[{"id":"leaf","width":40,"height":30}]}]}]
                }))
                .unwrap();
                let (arena, root) = LayeredArena::from_elk_graph(&graph).unwrap();
                assert_eq!(arena.graphs[root as usize].direction, root_direction);
                let expected = if policy == "SEPARATE_CHILDREN" {
                    child_direction
                } else {
                    root_direction
                };
                assert_eq!(arena.graphs.len(), 3);
                for graph in &arena.graphs[1..] {
                    assert_eq!(
                        graph.direction, expected,
                        "{root_name}/{child_name}/{policy}"
                    );
                }
            }
        }
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_included_hierarchy_direction_complete_raw_output() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_included_hierarchy_direction.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 48);
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
        layout_elk_graph_with_layered_pipeline(
            &mut graph,
            ElkOptions {
                node_spacing: 20.0,
                edge_node_spacing: 10.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();
        assert_raw_graph(
            &row["expected"],
            &serde_json::to_value(&graph).unwrap(),
            name,
        );
    }
}

// This inventory contract covers every edge owner and immutable wire field.
// Full geometry differences for these32 source controls remain in the raw
// qualification report; this test does not count them as complete matches.
fn recursive_edge_inventory(graph: &Value, published: bool) -> Value {
    fn visit(node: &Value, owner: &mut Vec<String>, published: bool, rows: &mut Vec<Value>) {
        for (index, edge) in array(node, "edges").iter().enumerate() {
            let labels = array(edge, "labels")
                .iter()
                .map(|label| {
                    serde_json::json!({
                        "text":label.get("text"), "width":label.get("width"),
                        "height":label.get("height"), "layoutOptions":label.get("layoutOptions")
                    })
                })
                .collect::<Vec<_>>();
            let mut row = serde_json::json!({"owner":owner,"index":index,"id":edge["id"],
                "sources":edge["sources"],"targets":edge["targets"],
                "layoutOptions":edge.get("layoutOptions"),"labels":labels});
            if published {
                row["container"] = edge["container"].clone();
                row["sections"] = array(edge, "sections")
                    .iter()
                    .map(|section| {
                        serde_json::json!({
                            "id":section.get("id"),"incomingShape":section.get("incomingShape"),
                            "outgoingShape":section.get("outgoingShape")
                        })
                    })
                    .collect();
            }
            rows.push(row);
        }
        for child in array(node, "children") {
            owner.push(child["id"].as_str().unwrap().to_owned());
            visit(child, owner, published, rows);
            owner.pop();
        }
    }
    let mut rows = Vec::new();
    visit(
        graph,
        &mut vec![graph["id"].as_str().unwrap().to_owned()],
        published,
        &mut rows,
    );
    Value::Array(rows)
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_recursive_edge_identity_ownership_and_metadata_all_controls() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_owned_edges.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 32);
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
        assert_raw_graph(
            &recursive_edge_inventory(&row["input"], false),
            &recursive_edge_inventory(&serde_json::to_value(&graph).unwrap(), false),
            name,
        );
        let (arena, _) = crate::layered::LayeredArena::from_elk_graph(&graph).unwrap();
        assert_eq!(
            arena.edges.len(),
            recursive_edge_inventory(&row["input"], false)
                .as_array()
                .unwrap()
                .len(),
            "{name} imported edge count"
        );
        layout_elk_graph_with_layered_pipeline(
            &mut graph,
            ElkOptions {
                node_spacing: 20.0,
                edge_node_spacing: 10.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();
        assert_raw_graph(
            &recursive_edge_inventory(&row["expected"], true),
            &recursive_edge_inventory(&serde_json::to_value(&graph).unwrap(), true),
            name,
        );
    }
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_curated_compound_recursive_edges_complete_raw_output() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_owned_edge_compound.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 1);
    for row in rows {
        let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
        layout_elk_graph_with_layered_pipeline(
            &mut graph,
            ElkOptions {
                node_spacing: 20.0,
                edge_node_spacing: 10.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();
        assert_raw_graph(
            &row["expected"],
            &serde_json::to_value(&graph).unwrap(),
            "original09_compound",
        );
    }
}

#[test]
fn recursive_edge_bindings_restore_on_layout_error_and_unwind() {
    use serde_json::json;
    let mut graph: ElkGraph = serde_json::from_value(json!({"id":"root","layoutOptions":{},
        "children":[{"id":"group","width":0,"height":0,
            "children":[{"id":"leaf","width":40,"height":30}],
            "edges":[{"id":"same","sources":["leaf"],"targets":["missing"]}]}],
        "edges":[{"id":"same","sources":["group"],"targets":["group"]}]
    }))
    .unwrap();
    let before = serde_json::to_value(&graph).unwrap();
    assert!(layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).is_err());
    assert_eq!(serde_json::to_value(&graph).unwrap(), before);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut bindings = crate::model::EdgeBindings::take(&mut graph);
        assert_eq!(bindings.edges.len(), 2);
        bindings.edges[1].id = "changed-in-place".into();
        panic!("exercise edge-binding unwind restoration");
    }));
    assert!(outcome.is_err());
    assert_eq!(graph.edges[0].id, "same");
    assert_eq!(graph.children[0].edges[0].id, "changed-in-place");
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_center_label_sides_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_center_label_sides.json"),
        192,
    );
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_inline_label_stacks_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_inline_label_stacks.json"),
        48,
    );
}

fn compare_complete_source_rows(fixtures: &str, expected_count: usize) {
    let rows: Vec<Value> = serde_json::from_str(fixtures).unwrap();
    assert_eq!(rows.len(), expected_count);
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
        layout_elk_graph_with_layered_pipeline(
            &mut graph,
            ElkOptions {
                node_spacing: 20.0,
                edge_node_spacing: 10.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();
        assert_raw_graph(
            &row["expected"],
            &serde_json::to_value(&graph).unwrap(),
            name,
        );
    }
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_recursive_owned_edges_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_owned_edges.json"),
        32,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_nested_resize_depths_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_hierarchy_resize_depths.json"),
        192,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_cross_hierarchy_boundary_orders_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_hierarchy_boundary_orders.json"),
        256,
    );
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_curated_cross_hierarchy_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_curated_cross_hierarchy.json"),
        1,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_component_world_packing_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_component_world_packing.json"),
        432,
    );
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_root_list_presence_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_root_list_presence.json"),
        54,
    );
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_curated_component_direction_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_curated_component_direction.json"),
        1,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_external_port_spacing_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_external_port_spacing.json"),
        176,
    );
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_curated_single_container_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_curated_single_container.json"),
        1,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_nested_model_order_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_nested_input_model_sort.json"),
        480,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_label_pair_spacing_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_label_pair_spacing.json"),
        64,
    );
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_curated_isomorphic_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_curated_isomorphic.json"),
        1,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_native_coffman_graham_controls_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_native_coffman_graham_controls.json"
        ),
        624,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_coffman_option_forms_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_coffman_option_forms.json"),
        72,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_component_single_transfer_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_component_single_transfer.json"),
        128,
    );
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_curated_coffman_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_curated_coffman.json"),
        1,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_dense_component_trials_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_dense_component_trials.json"),
        128,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_component_trial_limits_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_component_trial_limits.json"),
        320,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_p1_component_discovery_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_p1_component_discovery.json"),
        128,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_component_lifecycle_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_component_lifecycle.json"),
        64,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_mixed_component_lifecycles_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_mixed_component_lifecycles.json"),
        128,
    );
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_label_identity_complete_raw_output() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_label_identity.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 30);
    for row in rows {
        let mut graph: ElkGraph = serde_json::from_str(row["raw_input"].as_str().unwrap()).unwrap();
        layout_elk_graph_with_layered_pipeline(
            &mut graph,
            ElkOptions {
                node_spacing: 20.0,
                edge_node_spacing: 10.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();
        assert_raw_graph(
            &row["expected"],
            &serde_json::to_value(graph).unwrap(),
            row["name"].as_str().unwrap(),
        );
    }
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_label_identity_invalid_type_and_integer_lexemes() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_invalid_label_identity.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 8);
    for row in rows {
        let error =
            serde_json::from_str::<ElkGraph>(row["raw_input"].as_str().unwrap()).unwrap_err();
        let source = row["source_error"]
            .as_str()
            .unwrap()
            .strip_prefix("decode ELK graph: ")
            .unwrap()
            .trim_end();
        assert!(
            error.to_string().starts_with(source),
            "{}: {error}",
            row["name"]
        );
    }
}

#[test]
fn complete_raw_contract_keeps_integer_identity_beyond_f64_precision() {
    for (a, b) in [
        (serde_json::json!(i64::MAX), serde_json::json!(i64::MAX - 1)),
        (serde_json::json!(42), serde_json::json!(42.0)),
    ] {
        assert!(std::panic::catch_unwind(|| assert_raw_graph(&a, &b, "$.labels[0].id")).is_err());
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_shared_port_alignment_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_shared_port_alignment.json"),
        96,
    );
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_junction_admission_and_stale_reset_complete_raw_output() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_junction_admission.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 10);
    for row in rows {
        let mut graph: ElkGraph = serde_json::from_str(row["raw_input"].as_str().unwrap()).unwrap();
        layout_elk_graph_with_layered_pipeline(
            &mut graph,
            ElkOptions {
                node_spacing: 20.0,
                edge_node_spacing: 10.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();
        assert_raw_graph(
            &row["expected"],
            &serde_json::to_value(graph).unwrap(),
            row["name"].as_str().unwrap(),
        );
    }
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_invalid_junction_slice_and_point_admission() {
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_invalid_junction_admission.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 6);
    for row in rows {
        assert!(
            serde_json::from_str::<ElkGraph>(row["raw_input"].as_str().unwrap()).is_err(),
            "{}",
            row["name"]
        );
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_north_south_switch_sides_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_north_south_switch_sides.json"),
        64,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_hierarchy_junctions_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_hierarchy_junctions.json"),
        64,
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_hierarchy_labels_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_hierarchy_labels.json"),
        128,
    );
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_ancestor_descendant_edges_complete_raw_output() {
    // Raw elk-go v0.2.0 output for the D2 wires of todo/container_child_edge,
    // todo/child_parent_edges and stable/chaos1: compoundPortNetFlow sides,
    // import-order splitting, moveCrossHierarchyLabels scope and
    // Transferrer.edgeLayoutOffsets frames for ancestor/descendant edges.
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_ancestor_descendant_edges.json"),
        3,
    );
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_coordinate_ancestor_cases_complete_raw_output() {
    compare_complete_source_rows(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_coordinate_ancestor_cases.json"),
        5,
    );
}
