// SPDX-License-Identifier: MPL-2.0
use std::{
    fs,
    io::Write,
    process::{Command, Output, Stdio},
};

fn run(source: &str, args: &[&str], mode: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .env("D2_LSP_MODE", mode)
        .args(args)
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

#[test]
fn lsp_mode_returns_partial_ast_before_compilation() {
    // Pinned d2cli.compile's LspOutputData: Parse, JSON, then os.Exit(42).
    let empty = run("", &["-", "-"], "1");
    assert_eq!(empty.status.code(), Some(42));
    assert_eq!(
        empty.stdout,
        br#"{"Ast":{"range":"-,0:0:0-0:0:0","nodes":null},"Err":null}"#
    );
    assert!(empty.stderr.is_empty());
    for source in [
        "a: {",
        "a: {shape: invalid}",
        "...@missing",
        "a: \"<>&\u{2028}\u{2029}雪\"",
    ] {
        let output = run(
            source,
            &["--sketch", "--isometric", "--layout=missing", "-", "-"],
            "1",
        );
        assert_eq!(output.status.code(), Some(42), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json["Ast"]["nodes"].is_array());
        assert_eq!(json["Err"].is_null(), source != "a: {");
        if source == "a: {" {
            assert_eq!(
                json["Err"]["errs"][0]["errmsg"],
                "-:1:4: maps must be terminated with }"
            );
        }
        if source.contains('<') {
            let raw = String::from_utf8(output.stdout).unwrap();
            assert!(raw.contains("\\u003c\\u003e\\u0026\\u2028\\u2029雪"));
            // CLI parse positions count UTF-8 bytes, as d2lib.Parse does.
            assert_eq!(
                json["Ast"]["range"],
                format!("-,0:0:0-0:{0}:{0}", source.len())
            );
        }
    }
    let ordinary = run("a: {shape: invalid}", &["-", "-"], "true");
    assert_eq!(ordinary.status.code(), Some(1));
    assert!(ordinary.stdout.is_empty());
}

#[test]
fn lsp_mode_uses_absolute_file_ranges_without_writing_outputs() {
    let root = std::env::temp_dir().join(format!("d2-lsp-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    // The child inherits a $PWD that is not its working directory, so Go's
    // os.Getwd (and filepath.Abs) fall back to getcwd, which resolves
    // symlinks such as macOS's /var -> /private/var. (Windows canonical
    // paths gain a \\?\ prefix that getcwd does not report.)
    #[cfg(unix)]
    let root = root.canonicalize().unwrap();
    fs::write(root.join("input.d2"), "...@missing").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .current_dir(&root)
        .env("D2_LSP_MODE", "1")
        .args(["./input.d2", "output.png"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(42), "{output:?}");
    assert!(output.stderr.is_empty());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let path = root.join("input.d2");
    assert!(
        json["Ast"]["range"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{},", path.display()))
    );
    assert!(json["Err"].is_null());
    assert!(!root.join("output.png").exists());
    fs::remove_dir_all(root).unwrap();
}
