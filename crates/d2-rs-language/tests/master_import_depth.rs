// SPDX-License-Identifier: MPL-2.0
//! Master d2ir import depth/public context calls and active-stack invariants.
#[allow(dead_code)]
#[path = "../examples/master-ir-scoreboard.rs"]
mod adapter;

use d2_rs_core::ast::{self, Shared};
use d2_rs_language::{
    compiler::imports::{ImportState, MAX_IMPORT_DEPTH},
    parser::ParseError,
};

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_master_import_calls() {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(|| {
            let corpus: serde_json::Value =
                serde_json::from_str(d2_rs_testdata::oracle_fixture!(
                    "tests/fixtures/master-import-depth.json"
                ))
                .unwrap();
            let cases = corpus["cases"].as_array().unwrap();
            assert_eq!(cases.len(), 5);
            for case in cases {
                assert_eq!(
                    adapter::evaluate(case["request"].clone()),
                    case["got"],
                    "{}",
                    case["name"]
                );
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

fn import(name: &str) -> Shared<ast::Import> {
    Shared::new(ast::Import {
        path: ast::KeyPath::from_strings([name]).path,
        ..Default::default()
    })
}

#[test]
fn depth_precedes_cycles_and_never_pushes_rejected_path() {
    let mut state = ImportState {
        stack: (0..=MAX_IMPORT_DEPTH)
            .map(|i| format!("level-{i}.d2"))
            .collect(),
        ..Default::default()
    };
    let original = state.stack.clone();
    let mut errors = ParseError::default();
    assert_eq!(state.push(import("level-0"), &mut errors), None);
    assert_eq!(state.stack, original);
    assert_eq!(
        errors.errors.unwrap()[0].message,
        "1:1: maximum import depth of 128 exceeded"
    );
    state.pop();
    let mut errors = ParseError::default();
    assert!(state.push(import("leaf"), &mut errors).is_some());
    assert!(errors.is_empty());
}

#[test]
fn import_depth_counts_active_chain_and_excludes_root() {
    let mut state = ImportState::default();
    let mut errors = ParseError::default();
    assert!(state.push(import("root"), &mut errors).is_some());
    for _ in 0..2 {
        for depth in 0..MAX_IMPORT_DEPTH {
            assert!(
                state
                    .push(import(&format!("child-{depth}")), &mut errors)
                    .is_some()
            );
        }
        for _ in 0..MAX_IMPORT_DEPTH {
            state.pop();
        }
        assert_eq!(state.stack.len(), 1);
    }
    assert!(errors.is_empty());
}
