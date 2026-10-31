// SPDX-License-Identifier: MPL-2.0
//! Replay input-only requests from the original grid resource-limit tests.
use super::*;
use d2_rs_core::Node;
use serde_json::{Value, json};

fn evaluate(call: &Value) -> Value {
    let rows = call["rows"].as_u64().unwrap() as usize;
    let columns = call["columns"].as_u64().unwrap() as usize;
    let count = call["objects"].as_u64().unwrap() as usize;
    match call["operation"].as_str().unwrap() {
        "capacity" => match d2_rs_core::grid::grid_capacity(rows as i64, columns as i64) {
            Ok(capacity) => json!({"error":"", "capacity":capacity}),
            Err(error) => json!({"error":error, "capacity":0}),
        },
        "construct" => {
            let (rows, columns) = match grid_dimensions(Some(rows), Some(columns), count, true) {
                Ok(dimensions) => dimensions,
                Err(error) => return json!({"error":error.to_string()}),
            };
            let (occupied_rows, occupied_columns) =
                grid_storage_dimensions(rows, columns, count, true);
            json!({"error":"", "rows":rows,"columns":columns,"occupied_rows":occupied_rows,"occupied_columns":occupied_columns})
        }
        "layout" => {
            let mut graph = Graph::default();
            graph.root_grid_rows = Some(rows);
            graph.root_grid_columns = Some(columns);
            // Expose the grid content size without synthetic-root padding.
            graph.root_horizontal_gap = Some(DEFAULT_GAP);
            graph.root_vertical_gap = Some(DEFAULT_GAP);
            for i in 0..count {
                graph.add_node(Node::new(
                    i.to_string(),
                    Size {
                        width: 100.,
                        height: 50.,
                    },
                ));
            }
            match layout(&graph, &LayoutOptions::default()) {
                Err(error) => json!({"error":error.to_string()}),
                Ok(result) => {
                    let size = result.synthetic_root.unwrap().size;
                    json!({"error":"", "width":size.width - 2.*DEFAULT_GAP, "height":size.height - 2.*DEFAULT_GAP})
                }
            }
        }
        _ => panic!("unknown operation"),
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_grid_resource_limits() {
    let fixture: Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/grid-capacity-master.json")).unwrap();
    let calls = fixture["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 5);
    for call in calls {
        let actual = evaluate(&call["input"]);
        if call["input"]["operation"] == "layout" {
            assert_eq!(actual["error"], call["result"]["error"]);
            for dimension in ["width", "height"] {
                assert_eq!(
                    actual[dimension].as_f64().unwrap().to_bits(),
                    call["result"][dimension].as_f64().unwrap().to_bits()
                );
            }
            assert_eq!(
                actual.as_object().unwrap().len(),
                call["result"].as_object().unwrap().len()
            );
        } else {
            assert_eq!(actual, call["result"], "{}", call["test"]);
        }
        // The same guard must run through the full native layout entry point.
        if call["input"]["operation"] == "construct" && call["result"]["error"] != "" {
            let mut input = call["input"].clone();
            input["operation"] = json!("layout");
            assert_eq!(evaluate(&input), call["result"]);
        }
    }
}

#[test]
fn expansion_is_checked_and_empty_cells_do_not_allocate() {
    assert_eq!(grid_dimensions(Some(2), Some(3), 7, true).unwrap(), (3, 3));
    assert_eq!(grid_dimensions(Some(2), Some(3), 7, false).unwrap(), (2, 4));
    assert_eq!(grid_storage_dimensions(1000, 1000, 1, true), (1, 1));
    assert_eq!(grid_storage_dimensions(1000, 1000, 1, false), (1, 1));
    assert_eq!(grid_storage_dimensions(1000, 1000, 0, true), (0, 0));
    assert_eq!(grid_storage_dimensions(3, 3, 7, true), (3, 3));
    assert!(grid_dimensions(Some(1000), Some(1001), 1, true).is_err());
    assert!(
        grid_dimensions(Some(1), Some(10000), 10001, false)
            .unwrap_err()
            .to_string()
            .starts_with("expanded grid: ")
    );
    assert!(grid_dimensions(Some(1), None, 1000001, true).is_err());
}

#[test]
#[ignore = "executed explicitly by grid-capacity-scoreboard.py"]
fn replay() {
    let calls: Vec<Value> = serde_json::from_slice(
        &std::fs::read(std::env::var_os("D2RS_GRID_CAPACITY_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let results: Vec<Value> = calls
        .iter()
        .map(|call| {
            std::panic::catch_unwind(|| evaluate(call)).unwrap_or_else(|_| json!({"panic":true}))
        })
        .collect();
    std::fs::write(
        std::env::var_os("D2RS_GRID_CAPACITY_RESPONSE").unwrap(),
        serde_json::to_vec(&results).unwrap(),
    )
    .unwrap();
}
