// SPDX-License-Identifier: MPL-2.0
//! Cache-layout and stamp-verification contract (no real fixtures needed).
use d2_rs_testdata::{Problem, locate_in, read_verified};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("d2-rs-testdata")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("ws/crates/demo")).unwrap();
    dir
}

fn stamp(logical: &str, bytes: &[u8]) -> String {
    let sha: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!(
        r#"{{"logical_path":"{logical}","size":{},"sha256":"{sha}","key":"k"}}"#,
        bytes.len()
    )
}

#[test]
fn logical_path_is_crate_relative_and_verified_against_stamp() {
    let dir = scratch("verified");
    let ws = dir.join("ws");
    let cache = dir.join("cache");
    let located = locate_in(
        &cache,
        &ws,
        &ws.join("crates/demo"),
        "tests/fixtures/a.json",
    )
    .unwrap();
    assert_eq!(located.logical, "crates/demo/tests/fixtures/a.json");
    assert_eq!(
        located.artifact,
        cache.join("crates/demo/tests/fixtures/a.json")
    );

    let err = read_verified(&located).unwrap_err();
    assert_eq!(err.problem, Problem::MissingArtifact);
    let message = err.to_string();
    assert!(message.contains(
        "uv run --no-project python scripts/regen-fixtures.py --only crates/demo/tests/fixtures/a.json"
    ));

    fs::create_dir_all(located.artifact.parent().unwrap()).unwrap();
    fs::write(&located.artifact, b"{}").unwrap();
    assert_eq!(
        read_verified(&located).unwrap_err().problem,
        Problem::MissingStamp
    );

    fs::write(&located.stamp, stamp(&located.logical, b"{}")).unwrap();
    assert_eq!(read_verified(&located).unwrap(), b"{}");

    fs::write(&located.artifact, b"[]").unwrap();
    let err = read_verified(&located).unwrap_err();
    assert!(matches!(err.problem, Problem::Mismatch(_)));
    assert!(err.to_string().contains("--force"));

    fs::write(&located.stamp, stamp("crates/other/a.json", b"[]")).unwrap();
    assert!(matches!(
        read_verified(&located).unwrap_err().problem,
        Problem::Mismatch(_)
    ));
    fs::write(&located.stamp, "not json").unwrap();
    assert!(matches!(
        read_verified(&located).unwrap_err().problem,
        Problem::BadStamp(_)
    ));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rejects_escaping_paths_and_foreign_crates() {
    let dir = scratch("paths");
    let ws = dir.join("ws");
    let crate_dir = ws.join("crates/demo");
    for path in ["../x.json", "/abs.json", ""] {
        let err = locate_in(&dir, &ws, &crate_dir, path).unwrap_err();
        assert!(matches!(err.problem, Problem::BadPath(_)), "{path}");
    }
    let err = locate_in(&dir, &crate_dir, &dir, "a.json").unwrap_err();
    assert!(matches!(err.problem, Problem::BadPath(_)));
    fs::remove_dir_all(&dir).unwrap();
}
