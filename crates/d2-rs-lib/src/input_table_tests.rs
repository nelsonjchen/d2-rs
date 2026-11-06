// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::wire_geometry_tests::{geometry, run};
use super::*;

#[test]
fn complete_input_table_source_contract() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/input-tables-source-v090.json"
    ))
    .unwrap();
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 1300);
    let mut differences = Vec::new();
    let mut successes = 0;
    let mut results = Vec::new();
    for row in rows {
        let result = run(&row["input"]);
        let error = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
        let name = &row["input"]["name"];
        if error != row["error"] {
            differences.push(format!(
                "{name}: error {error:?}, expected {}",
                row["error"]
            ));
        } else if let Ok(actual) = &result {
            successes += 1;
            if geometry(actual) != geometry(&row["graph"]) {
                differences.push(format!(
                    "{name}: geometry {} != {}",
                    geometry(actual),
                    geometry(&row["graph"])
                ));
            }
        }
        // Go's adapter stores Object.Text().FontSize minus the class/table
        // header addend. Compare every decoded object, including failures at
        // a later engine validation stage, rather than inferring this from
        // geometry alone. Failed Go deserializations have no font inventory.
        let fonts = if !row["effective_font_sizes"].as_object().unwrap().is_empty() {
            match decode_graph(&row["input"]["graph"], false) {
                Ok((graph, _, _)) => {
                    let fonts: BTreeMap<_, _> = graph
                        .nodes()
                        .map(|(_, n)| (n.external_id.clone(), n.font_size))
                        .collect();
                    let actual = json!(fonts);
                    if actual != row["effective_font_sizes"] {
                        differences.push(format!(
                            "{name}: effective fonts {actual} != {}",
                            row["effective_font_sizes"]
                        ));
                    }
                    Some(actual)
                }
                Err(error) => {
                    differences.push(format!("{name}: decode for font inventory: {error}"));
                    None
                }
            }
        } else {
            None
        };
        results.push(
            json!({"name":name,"error":error,"graph":result.ok(),"effective_font_sizes":fonts}),
        );
    }
    if let Some(path) = std::env::var_os("D2_RS_TEST_INPUT_TABLES_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    assert!(
        differences.is_empty(),
        "{} differences:\n{}",
        differences.len(),
        differences.join("\n")
    );
    assert_eq!(successes, 470);
}
