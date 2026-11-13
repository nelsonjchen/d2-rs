// SPDX-License-Identifier: MPL-2.0
//! TALA adapter input limits (d2talalayout/adapter_validate.go,
//! collectD2ObjectVisits). Expected stderr and exit codes are from the pinned
//! Go D2 (6a7b4837) CLI for the same stdin input. Seed-worker stack depth is
//! covered by a d2-rs-tala unit test; a depth-256 CLI run is too slow
//! unoptimized.
use std::io::Write;
use std::process::{Command, Stdio};

fn run(layout: &str, extra: &[&str], source: &str) -> std::process::Output {
    let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .arg(format!("--layout={layout}"))
        .args(extra)
        .args(["-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    process
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    process.wait_with_output().unwrap()
}

/// `a.a.a...` with `depth` levels below the root.
fn nested(depth: usize) -> String {
    let mut source = vec!["a"; depth].join(".");
    source.push('\n');
    source
}

fn assert_error(output: &std::process::Output, expected: &str) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty());
    // Go and d2-rs both print a progress line every five seconds; it depends
    // only on wall time, so slow (unoptimized) test builds may emit it.
    let stderr: String = String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| *line != "info: compiling & running layout algorithms...")
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(stderr, expected);
}

#[test]
fn tala_rejects_nesting_deeper_than_256() {
    // collectD2ObjectVisits rejects depth > maxInputTreeDepth before any
    // recursive layout stage runs.
    for depth in [257, 300, 1000] {
        assert_error(
            &run("tala", &[], &nested(depth)),
            "err: failed to compile -: D2 object nesting depth exceeds limit 256\n",
        );
    }
}

#[test]
fn tala_rejects_more_than_10000_objects() {
    let source: String = (0..10_001).map(|i| format!("n{i}\n")).collect();
    assert_error(
        &run("tala", &[], &source),
        "err: failed to compile -: D2 object count exceeds limit 10000\n",
    );
}
