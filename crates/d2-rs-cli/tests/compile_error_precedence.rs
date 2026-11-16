// SPDX-License-Identifier: MPL-2.0
use std::{fs, process::Command};

#[test]
fn explicit_source_options_preserve_compile_error_precedence() {
    // Pinned d2cli/main.go compile calls d2lib.Compile before checking the
    // sketch/isometric conflict, regardless of which flags were supplied.
    let root = std::env::temp_dir().join(format!("d2-compile-order-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    for (source, expected) in [
        (
            "a: {shape: package; style.3d: true}",
            "input.d2:1:21: key \"3d\" can only be applied to squares, rectangles, and hexagons",
        ),
        (
            "a -> b",
            "sketch cannot be combined with isometric rendering",
        ),
    ] {
        fs::write(root.join("input.d2"), source).unwrap();
        for (explicit, engine) in [(false, "elk"), (true, "elk"), (true, "missing")] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_d2-rs"));
            command.current_dir(&root).args(["--sketch", "--isometric"]);
            if explicit {
                command.args([
                    "--bundle=false",
                    "--tala-seeds=1",
                    "--theme=301",
                    "--dark-theme=105",
                    "--pad=0",
                    "--center",
                ]);
            }
            command.arg(format!("--layout={engine}"));
            let result = command.args(["input.d2", "output.svg"]).output().unwrap();
            assert_eq!(result.status.code(), Some(1));
            let error = String::from_utf8(result.stderr).unwrap();
            assert!(error.contains(expected), "{error}");
            assert!(result.stdout.is_empty());
            assert!(!root.join("output.svg").exists());
        }
    }
    fs::remove_dir_all(root).unwrap();
}
