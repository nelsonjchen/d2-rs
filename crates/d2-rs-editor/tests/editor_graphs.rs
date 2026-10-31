// SPDX-License-Identifier: MPL-2.0
//! Complete native editor contracts from every live pinned d2oracle golden.
//! Inputs and expected graphs are source-oracle data, not native snapshots.
use d2_rs_editor::*;
use serde_json::json;

// Oracle tier: the regenerated editor-graphs artifact (recipe
// mathtext-txt-editor-graphs, docs/FIXTURES.md).
#[cfg(feature = "oracle-fixtures")]
mod source_goldens {
    use super::*;
    use serde::Deserialize;
    use serde_json::Value;
    use std::collections::HashMap;

    #[derive(Deserialize)]
    struct Fixture {
        upstream_commit: String,
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        name: String,
        request: Request,
        error: String,
        expected: Value,
    }

    #[derive(Deserialize)]
    struct Request {
        operation: String,
        input: String,
        path: String,
        key: String,
        expected_text: String,
        #[serde(default)]
        new_key: String,
        files: Option<HashMap<String, String>>,
        board_path: Option<Vec<String>>,
        #[serde(default)]
        include_descendants: bool,
        tag: Option<String>,
        value: Option<String>,
        #[serde(default)]
        src: String,
        #[serde(default)]
        dst: String,
    }

    fn check(operation: &str, expected_count: usize) {
        let fixture: Fixture =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!(
                "tests/fixtures/editor-graphs-v090.json"
            ))
            .unwrap();
        let manifest: Value =
            serde_json::from_str(include_str!("../../../reference-manifest.json")).unwrap();
        assert_eq!(fixture.upstream_commit, manifest["d2"]["commit"].as_str().unwrap());
        assert_eq!(fixture.cases.len(), 426);
        let mut count = 0;
        for case in fixture
            .cases
            .into_iter()
            .filter(|c| c.request.operation == operation)
        {
            count += 1;
            let r = case.request;
            let options = MoveOptions {
                path: r.path,
                files: r.files.unwrap_or_default(),
                board_path: r.board_path.unwrap_or_default(),
            };
            let actual =
                match operation {
                    "create" => create_object_with_options(&r.input, &r.key, &options)
                        .map(|v| (v.source, v.graph)),
                    "set" => set_object_with_options(
                        &r.input,
                        &r.key,
                        r.tag.as_deref(),
                        r.value.as_deref(),
                        &options,
                    )
                    .map(|v| (v.source, v.graph)),
                    "delete" => delete_object_with_options(&r.input, &r.key, &options)
                        .map(|v| (v.source, v.graph)),
                    "move" => move_object_with_options(
                        &r.input,
                        &r.key,
                        &r.new_key,
                        r.include_descendants,
                        &options,
                    )
                    .map(|v| (v.source, v.graph)),
                    "rename" => rename_object_with_options(&r.input, &r.key, &r.new_key, &options)
                        .map(|v| (v.source, v.graph)),
                    "reconnect" => reconnect_edge_with_options(
                        &r.input,
                        &r.key,
                        (!r.src.is_empty()).then_some(r.src.as_str()),
                        (!r.dst.is_empty()).then_some(r.dst.as_str()),
                        &options,
                    )
                    .map(|v| (v.source, v.graph)),
                    _ => unreachable!(),
                };
            let value = match actual {
                Ok((source, graph)) => {
                    assert!(case.error.is_empty(), "{} expected an error", case.name);
                    assert_eq!(source, r.expected_text, "{} source", case.name);
                    json!({"graph":graph,"err":"<nil>"})
                }
                Err(error) => {
                    assert_eq!(error.to_string(), case.error, "{} error", case.name);
                    // Upstream edit_test.go uses Go's debug representation in its
                    // graph golden and the ordinary error string for text parity.
                    let debug = match error {
                        GraphEditError::Reconnect(message) => format!(
                            "&errors.errorString{{s:{}}}",
                            d2_rs_core::quote_go_bytes(message.as_bytes())
                        ),
                        GraphEditError::OutsideScope => "d2oracle.OutsideScopeError{}".into(),
                        other => other.to_string(),
                    };
                    json!({"graph":null,"err":debug})
                }
            };
            assert_eq!(value, case.expected, "{} complete graph", case.name);
        }
        assert_eq!(count, expected_count, "{operation} inventory");
    }

    #[test]
    fn create_complete_graphs() {
        check("create", 36);
    }
    #[test]
    fn set_complete_graphs() {
        check("set", 102);
    }
    #[test]
    fn delete_complete_graphs() {
        check("delete", 122);
    }
    #[test]
    fn move_complete_graphs() {
        check("move", 117);
    }
    #[test]
    fn rename_complete_graphs() {
        check("rename", 25);
    }
    #[test]
    fn reconnect_complete_graphs() {
        check("reconnect", 24);
    }
}

#[test]
fn create_returns_the_compiled_unique_key() {
    let object = create_object("x\nx 2\n", "x 2").unwrap();
    assert_eq!(object.key, "x 3");
    assert_eq!(object.source, "x\nx 2\nx 3\n");
    let edge = create_object("a -> b\n", "a -> b").unwrap();
    assert_eq!(edge.key, "(a -> b)[1]");
    assert_eq!(edge.source, "a -> b\na -> b\n");
}

#[test]
fn legacy_source_edit_serialization_is_preserved() {
    let moved = move_object("x\n", "x", "y", false).unwrap();
    assert!(!moved.graph.is_null());
    assert_eq!(
        serde_json::to_value(moved).unwrap(),
        json!({"source":"y\n","key":"y"})
    );
    let renamed = rename_object("x\n", "x", "y").unwrap();
    assert_eq!(
        serde_json::to_value(renamed).unwrap(),
        json!({"source":"y\n","key":"y"})
    );
    let set = set_object("x\n", "x", None, Some("label")).unwrap();
    assert_eq!(
        serde_json::to_value(set).unwrap(),
        json!({"source":"x: label\n"})
    );
    let deleted = delete_object("x\n", "x").unwrap();
    assert_eq!(serde_json::to_value(deleted).unwrap(), json!({"source":""}));
}
