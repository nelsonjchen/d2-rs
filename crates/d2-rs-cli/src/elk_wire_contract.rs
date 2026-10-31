// SPDX-License-Identifier: MPL-2.0
//! Test-only product-wire checks; no production tracing or trace arguments.
use super::*;

#[test]
fn source_elk_compiler_wire_contract() {
    let (graph, _, _, _) = source_layout(b"a -> b", &["--engine=elk".into()]).unwrap();
    let wire =
        d2_rs_elk::graph_to_elk_graph(&graph, &graph.node_labels, d2_rs_elk::ElkOptions::default());
    assert_eq!(
        wire.children
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(wire.edges.len(), 1);
    assert_eq!(wire.edges[0].sources, vec!["a"]);
    assert_eq!(wire.edges[0].targets, vec!["b"]);
    assert_eq!(wire.layout_options["elk.direction"], "DOWN");
    assert_eq!(wire.edges[0].id, "(a -> b)[0]");
    let mut transport = wire.clone();
    let options = transport.children[0]
        .layout_options
        .as_mut()
        .unwrap()
        .as_object_mut()
        .unwrap();
    for key in [
        "d2-rs.marginTop",
        "d2-rs.marginRight",
        "d2-rs.marginBottom",
        "d2-rs.marginLeft",
    ] {
        options.insert(key.into(), serde_json::json!(41));
    }
    let (arena, _) = d2_rs_elk::LayeredArena::from_elk_graph(&transport).unwrap();
    assert_eq!(arena.nodes[0].margin_top, 0.0);
    assert_eq!(arena.nodes[0].margin_right, 0.0);
    assert!(!arena.nodes[0].margins_in_wire_dimensions);

    assert_eq!(
        wire.children[0].layout_options.as_ref().unwrap()["elk.direction"],
        ""
    );
    assert_eq!(
        wire.children[0].layout_options.as_ref().unwrap()["elk.spacing.nodeSelfLoop"],
        0
    );
    let (table, _, _, _) = source_layout(
        b"a: {shape: sql_table; id: int; name: text}\nb\na.id -> b",
        &["--engine=elk".into()],
    )
    .unwrap();
    assert_eq!(table.table_column_names[&NodeId(0)], vec!["id", "name"]);
    let sql =
        d2_rs_elk::graph_to_elk_graph(&table, &table.node_labels, d2_rs_elk::ElkOptions::default());
    assert_eq!(sql.children[0].ports[0].id, "a.id.src");
    assert_eq!(sql.edges[0].sources, vec!["a.id.src"]);

    let (unusual,_,_,_)=source_layout(b"u: {shape: sql_table; \"__root__\": text; \"with space\": int}\nv\nu.\"__root__\" -> v\nu.\"with space\" -> v\nu -> v", &["--engine=elk".into()]).unwrap();
    let table = d2_rs_elk::graph_to_elk_graph(
        &unusual,
        &unusual.node_labels,
        d2_rs_elk::ElkOptions::default(),
    );
    assert_eq!(table.children[0].ports.len(), 5);
    assert_eq!(table.edges[0].sources, vec!["u.__root__.src"]);
    let (empty, _, _, _) = source_layout(
        b"u: {shape: sql_table}\nv\nu -> v",
        &["--engine=elk".into()],
    )
    .unwrap();
    let table =
        d2_rs_elk::graph_to_elk_graph(&empty, &empty.node_labels, d2_rs_elk::ElkOptions::default());
    assert_eq!(table.children[0].ports.len(), 1);

    if let Some(input) = std::env::var_os("D2_RS_TEST_ELK_WIRE_REQUESTS") {
        let requests: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
        let go_inputs = std::env::var_os("D2_RS_TEST_ELK_GO_INPUT").map(|path| {
            serde_json::from_slice::<Vec<serde_json::Value>>(&std::fs::read(path).unwrap()).unwrap()
        });
        let mut records = Vec::new();
        for row in requests {
            let hex = row["hex"].as_str().unwrap();
            assert_eq!(hex.len() % 2, 0);
            let bytes = hex
                .as_bytes()
                .chunks_exact(2)
                .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
                .collect::<Vec<_>>();
            // These frozen controls contain no grid/sequence/near projections.
            // ELK dispatch borrows the unchanged measured compiler graph.
            let (graph, _, _, _) = source_layout(
                &bytes,
                &[
                    "--engine=elk".into(),
                    format!("--path={}", row["path"].as_str().unwrap()),
                ],
            )
            .unwrap();
            let wire = d2_rs_elk::graph_to_elk_graph(
                &graph,
                &graph.node_labels,
                d2_rs_elk::ElkOptions::default(),
            );
            let mut own = wire.clone();
            d2_rs_elk::layout_elk_graph_with_layered_pipeline(
                &mut own,
                d2_rs_elk::ElkOptions::default(),
            )
            .unwrap();
            let source_input = go_inputs.as_ref().map(|rows| {
                let item = rows
                    .iter()
                    .find(|item| item["name"] == row["name"])
                    .unwrap();
                let mut source: d2_rs_elk::ElkGraph =
                    serde_json::from_value(item["wire"].clone()).unwrap();
                d2_rs_elk::layout_elk_graph_with_layered_pipeline(
                    &mut source,
                    d2_rs_elk::ElkOptions::default(),
                )
                .unwrap();
                source
            });
            records.push(serde_json::json!({"name":row["name"],"wire":wire,"native_on_own_input":own,"native_on_go_input":source_input}));
        }
        let output = std::env::var_os("D2_RS_TEST_ELK_WIRE_OUTPUT").expect("capture output path");
        std::fs::write(output, serde_json::to_vec_pretty(&records).unwrap()).unwrap();
    }
}
