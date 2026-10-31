// SPDX-License-Identifier: MPL-2.0
#[path = "../examples/library-source-scoreboard.rs"]
mod adapter;

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_master_library_source_calls() {
    let calls: Vec<serde_json::Value> =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/library-source-api.json")).unwrap();
    assert_eq!(calls.len(), 7);
    for call in calls {
        assert_eq!(
            adapter::evaluate(&call["request"]),
            call["got"],
            "{}",
            call["name"]
        );
    }
}

#[test]
fn cancellation_precedes_invalid_limits_and_retains_cause() {
    use d2_rs_core::{CancellationToken, LayoutError};
    use d2_rs_lib::source::{SourceError, SourceOptions, compile_source_ir};
    let token = CancellationToken::new();
    token.cancel();
    let mut options = SourceOptions::default();
    options.limits.max_variable_expansion = -1;
    let error = compile_source_ir(b"unclosed: {", options, Some(&token))
        .err()
        .unwrap();
    assert!(matches!(
        error,
        SourceError::Context(LayoutError::Cancelled)
    ));
    assert!(
        std::error::Error::source(&error)
            .unwrap()
            .is::<LayoutError>()
    );
}

#[test]
fn explicit_import_capability_propagates_to_source_compilation() {
    use d2_rs_lib::source::{SourceOptions, compile_source_ir};
    let mut options = SourceOptions::default();
    options.import_opener = Some(Box::new(|path| {
        assert_eq!(path, "secret.d2");
        Ok(Box::new(std::io::Cursor::new(b"allowed".to_vec())))
    }));
    let (compiler, _) = compile_source_ir(b"...@secret", options, None).unwrap();
    assert!(compiler.errors.is_empty());
}

#[test]
fn cancellation_during_import_reaches_the_library_caller() {
    use d2_rs_core::{CancellationToken, LayoutError};
    use d2_rs_lib::source::{SourceError, SourceOptions, compile_source_ir};
    let token = CancellationToken::new();
    let opener_token = token.clone();
    let options = SourceOptions {
        import_opener: Some(Box::new(move |_| {
            opener_token.cancel();
            Ok(Box::new(std::io::Cursor::new(b"allowed".to_vec())))
        })),
        ..Default::default()
    };
    let error = compile_source_ir(b"...@secret", options, Some(&token))
        .err()
        .unwrap();
    assert!(matches!(
        error,
        SourceError::Context(LayoutError::Cancelled)
    ));
}
