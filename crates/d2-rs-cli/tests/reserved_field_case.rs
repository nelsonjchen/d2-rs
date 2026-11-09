// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn capitalized_simple_reserved_fields_do_not_create_objects() {
    // Pinned compileField uses strings.ToLower for SimpleReservedKeywords,
    // then compileReserved switches on the original spelling. These values
    // are consequently ignored, rather than becoming visible shape labels.
    for engine in ["dagre", "elk", "tala"] {
        for key in [
            "Label",
            "Shape",
            "Icon",
            "Constraint",
            "Tooltip",
            "Link",
            "Near",
            "Width",
            "Height",
            "Direction",
            "Top",
            "Left",
            "Grid-rows",
            "Grid-columns",
            "Grid-gap",
            "Vertical-gap",
            "Horizontal-gap",
            "Class",
            "Vars",
            "LİNK",
        ] {
            let source = format!("{key}: rectangle\n");
            let output = compile(&source, engine);
            assert!(output.status.success(), "{engine}: {source}");
            assert!(output.stdout.is_empty(), "{engine}: {source}");
            assert!(output.stderr.is_empty(), "{engine}: {source}");
        }
        // Holders and quoted names do not use the simple-keyword lookup.
        for key in ["Style", "Classes", "\"Width\""] {
            let source = format!("{key}: rectangle\n");
            let output = compile(&source, engine);
            assert!(output.status.success(), "{engine}: {source}");
            assert!(
                String::from_utf8(output.stdout)
                    .unwrap()
                    .contains(">rectangle</text>"),
                "{engine}: {source}"
            );
        }
    }
}

#[test]
fn shape_prepass_uses_go_equal_fold_and_preserves_field_diagnostic_order() {
    // Pinned d2compiler.compileMap uses d2ir.Map.GetField("shape") before
    // compileField, then skips only the literal unquoted lowercase spelling.
    // GetField uses strings.EqualFold, including the Unicode long s.
    let cases = [
        (
            "shape: { x }\n",
            "-:1:1: reserved field shape does not accept composite\n",
        ),
        (
            "Shape: { x }\n",
            "-:1:1: reserved field shape does not accept composite\nerr: -:1:1: reserved field Shape does not accept composite\n",
        ),
        (
            "SHAPE: { x }\n",
            "-:1:1: reserved field shape does not accept composite\nerr: -:1:1: reserved field SHAPE does not accept composite\n",
        ),
        (
            "ſhape: { x }\n",
            "-:1:1: reserved field shape does not accept composite\n",
        ),
        (
            "a: { Shape: { x } }\n",
            "-:1:6: reserved field shape does not accept composite\nerr: -:1:6: reserved field Shape does not accept composite\n",
        ),
        (
            "shape: { x }; Shape: { y }\n",
            "-:1:15: reserved field shape does not accept composite\n",
        ),
        (
            "Width: { x }; Shape: { y }\n",
            "-:1:15: reserved field shape does not accept composite\nerr: -:1:1: reserved field Width does not accept composite\nerr: -:1:15: reserved field Shape does not accept composite\n",
        ),
    ];
    for engine in ["dagre", "elk", "tala"] {
        for (source, diagnostic) in cases {
            assert_diagnostic(source, engine, diagnostic);
        }
        // Other capitalized reserved keywords use the regular compileField
        // path, with their authored spelling and exactly one diagnostic.
        for key in [
            "Label",
            "Width",
            "Height",
            "Direction",
            "Near",
            "Icon",
            "Tooltip",
            "Link",
            "Constraint",
            "Class",
        ] {
            assert_diagnostic(
                &format!("{key}: {{ x }}\n"),
                engine,
                &format!("-:1:1: reserved field {key} does not accept composite\n"),
            );
        }
    }
}

fn assert_diagnostic(source: &str, engine: &str, diagnostic: &str) {
    let output = compile(source, engine);
    assert_eq!(output.status.code(), Some(1), "{engine}: {source}");
    assert!(output.stdout.is_empty(), "{engine}: {source}");
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("err: failed to compile -: {diagnostic}"),
        "{engine}: {source}"
    );
}

fn compile(source: &str, engine: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args([&format!("--layout={engine}"), "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
