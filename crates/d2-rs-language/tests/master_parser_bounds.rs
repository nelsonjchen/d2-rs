// SPDX-License-Identifier: MPL-2.0
//! d2parser/parse_test.go master 6a7b48378, plus shared map/array and context contracts.
use d2_rs_core::LayoutError;
use d2_rs_language::{
    ParseContextError, ParseOptions, parse, parse_stream_with_context, parse_with_context,
    parser::{MAX_NESTING_DEPTH, ParseError},
};
use std::{cell::Cell, rc::Rc};

#[test]
fn original_nesting_boundaries_and_partial_ranges() {
    // A larger test stack keeps the host test runner separate from the D2 bound.
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(|| {
            for (kind, input, column) in [
                ("map", "a:{".repeat(129) + &"}".repeat(129), 387),
                (
                    "array",
                    "a:".to_owned() + &"[".repeat(129) + "x" + &"]".repeat(129),
                    131,
                ),
            ] {
                let output = parse("deep.d2", input.as_bytes(), None);
                let error = output.error.unwrap();
                assert_eq!(
                    error.to_string(),
                    format!("deep.d2:1:{column}: maximum nesting depth of 128 exceeded"),
                    "{kind}"
                );
                assert_eq!(
                    error.errors.unwrap().len(),
                    1,
                    "termination diagnostics suppressed after halt"
                );
                assert_eq!(output.map.range.end.byte, column);
                assert!(output.map.nodes.is_some(), "partial AST remains available");
            }
            let input = "a:{".repeat(MAX_NESTING_DEPTH) + &"}".repeat(MAX_NESTING_DEPTH);
            let output = parse("deep.d2", input.as_bytes(), None);
            assert!(output.error.is_none());
            assert_eq!(output.map.range.end.byte, 512);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn mixed_nesting_and_extreme_input_share_the_same_limit() {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(|| {
            let supported =
                "a:{".repeat(64) + "b:" + &"[".repeat(64) + "x" + &"]".repeat(64) + &"}".repeat(64);
            assert!(
                parse("mixed.d2", supported.as_bytes(), None)
                    .error
                    .is_none()
            );
            let overflow =
                "a:{".repeat(64) + "b:" + &"[".repeat(65) + "x" + &"]".repeat(65) + &"}".repeat(64);
            assert!(
                parse("mixed.d2", overflow.as_bytes(), None)
                    .error
                    .unwrap()
                    .to_string()
                    .contains("maximum nesting depth")
            );
            let extreme = "a:{".repeat(10000) + &"}".repeat(10000);
            let output = parse("extreme.d2", extreme.as_bytes(), None);
            assert_eq!(output.map.range.end.byte, 387);
            assert_eq!(output.error.unwrap().errors.unwrap().len(), 1);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn original_precanceled_parse_retains_typed_cause_and_empty_map() {
    let output = parse_with_context(
        "canceled.d2",
        b"x",
        None,
        Some(Box::new(|| Some(LayoutError::Cancelled))),
    );
    assert!(matches!(
        output.error,
        Some(ParseContextError::Context(LayoutError::Cancelled))
    ));
    assert_eq!(output.map.range.end.byte, 0);
    assert!(output.map.nodes.is_none());
}

#[test]
fn context_latches_and_has_precedence_over_syntax_errors_in_both_boundaries() {
    for streaming in [false, true] {
        let polls = Rc::new(Cell::new(0));
        let observed = polls.clone();
        let context = Box::new(move || {
            observed.set(observed.get() + 1);
            (observed.get() == 2).then_some(LayoutError::DeadlineExceeded)
        });
        let mut errors = ParseError::default();
        let options = Some(ParseOptions {
            utf16_positions: false,
            errors: Some(&mut errors),
        });
        let input = "};".to_owned() + &"x;".repeat(1000);
        let output = if streaming {
            parse_stream_with_context(
                "partial.d2",
                &mut std::io::Cursor::new(input.as_bytes()),
                options,
                Some(context),
            )
        } else {
            parse_with_context("partial.d2", input.as_bytes(), options, Some(context))
        };
        assert_eq!(
            output.error.as_ref().unwrap().to_string(),
            "context deadline exceeded"
        );
        assert!(matches!(
            output.error,
            Some(ParseContextError::Context(LayoutError::DeadlineExceeded))
        ));
        assert_eq!(polls.get(), 2, "latched cancellation does not poll again");
        assert!(
            !errors.is_empty(),
            "syntax accumulator survives context precedence"
        );
        assert!(output.map.nodes.is_some());
        assert!(output.map.range.end.byte > 0 && output.map.range.end.byte < input.len() as i64);
    }
}
