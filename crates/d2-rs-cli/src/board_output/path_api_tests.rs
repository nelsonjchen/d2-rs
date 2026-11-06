// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of original board_output_test.go path contracts.
use super::*;
use serde_json::{Value, json};

fn collect_boards(
    node: &Value,
    target: &str,
    identity: &str,
    boards: &mut Vec<(String, String, bool)>,
) {
    boards.push((
        target.into(),
        identity.into(),
        node["folder"].as_bool().unwrap(),
    ));
    for kind in ["layers", "scenarios", "steps"] {
        for child in node[kind].as_array().unwrap() {
            let name = child["name"].as_str().unwrap();
            let key = d2_rs_core::quote_go_bytes(name.as_bytes());
            let next = if target.is_empty() {
                format!("{kind}.{key}")
            } else {
                format!("{target}.{kind}.{key}")
            };
            collect_boards(child, &next, &format!("{identity}.{kind}.{name}"), boards);
        }
    }
}

fn observe(input: &Value) -> Value {
    match input["op"].as_str().unwrap() {
        "component" => {
            let bytes: Vec<u8> = serde_json::from_value(input["bytes"].clone()).unwrap();
            json!({"value": board_output_component(&bytes)})
        }
        "validate" => {
            let output = input["output"].as_str().unwrap();
            assert_eq!(
                input["diagram"]["name"], "",
                "original tests have unnamed roots"
            );
            // These original tests use SVG paths. Root adaptation is transport:
            // production board_output_path plans every board's relative path.
            let root = output.strip_suffix(".svg").expect("SVG output");
            let mut boards = Vec::new();
            collect_boards(&input["diagram"], "", "root", &mut boards);
            let targets: Vec<_> = boards
                .iter()
                .skip(1)
                .map(|(target, _, _)| target.clone())
                .collect();
            let outputs: Vec<_> = boards
                .iter()
                .filter(|(_, _, folder)| !folder)
                .map(|(target, identity, _)| {
                    (
                        identity.clone(),
                        Path::new(root).join(crate::board_output_path(
                            Some(target),
                            crate::ExportFormat::Svg,
                            &targets,
                        )),
                    )
                })
                .collect();
            json!({"error": validate_board_output_paths(&outputs).err().unwrap_or_default()})
        }
        "append_name" | "append_component" => {
            let output = input["output"].as_str().unwrap();
            // All original calls use SVG paths. Rust with_extension("")
            // retains a dot-only basename, as splitBoardOutputPath requires.
            assert!(output.ends_with(".svg"));
            let root = Path::new(output).with_extension("");
            let component = if input["op"] == "append_name" {
                board_output_component(input["name"].as_str().unwrap())
            } else {
                input["component"].as_str().unwrap().to_owned()
            };
            match contained_board_output_component(&root, &component) {
                Ok(mut path) => {
                    path.as_mut_os_string().push(".svg");
                    json!({"value": path, "error": ""})
                }
                Err(error) => json!({"value": "", "error": error}),
            }
        }
        "within" => json!({"error": ensure_path_within(
            Path::new(input["root"].as_str().unwrap()),
            Path::new(input["path"].as_str().unwrap()),
        ).err().unwrap_or_default()}),
        other => panic!("unknown path replay operation {other}"),
    }
}

#[test]
#[ignore = "input-only original API replay"]
fn replay() {
    let request = std::env::var_os("D2RS_PATH_REQUEST").expect("request");
    let response = std::env::var_os("D2RS_PATH_RESPONSE").expect("response");
    let inputs: Vec<Value> = serde_json::from_slice(&std::fs::read(request).unwrap()).unwrap();
    let results: Vec<_> = inputs.iter().map(observe).collect();
    std::fs::write(response, serde_json::to_vec(&results).unwrap()).unwrap();
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_board_path_contracts() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/board-path-api.json"
    ))
    .unwrap();
    let calls = fixture["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 75);
    for call in calls {
        let actual = observe(&call["input"]);
        assert_eq!(actual, call["result"], "{}", call["test"]);
    }
}
