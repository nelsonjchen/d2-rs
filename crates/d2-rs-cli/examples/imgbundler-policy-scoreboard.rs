// SPDX-License-Identifier: MPL-2.0
//! Raw filesystem fixture adapter. Reads, policy checks, probing, limits and
//! per-document caching run in the native libraries, consecutively in one process.
use base64::Engine as _;
use d2_rs_scenebuild::ImageLocalFiles;
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};

fn materialize(request: &Value) -> PathBuf {
    let root = PathBuf::from(std::env::var_os("D2RS_FIXTURE_ROOT").expect("fixture root"));
    let fixture = PathBuf::from(request["fixture"].as_str().unwrap());
    assert!(fixture.is_absolute() && fixture.starts_with(&root) && fixture != root);
    assert!(!fixture.components().any(|c| c == Component::ParentDir));
    assert!(!fixture.exists() && !fixture.is_symlink());
    std::fs::create_dir_all(&fixture).unwrap();
    let mut entries: Vec<_> = request["entries"].as_object().unwrap().iter().collect();
    entries.sort_by_key(|(name, _)| Path::new(name).components().count());
    for (name, entry) in entries {
        let relative = Path::new(name);
        assert!(
            !relative.is_absolute() && !relative.components().any(|c| c == Component::ParentDir)
        );
        let path = fixture.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        if entry["directory"] == true {
            std::fs::create_dir_all(path).unwrap();
        } else if let Some(target) = entry["link"].as_str() {
            std::os::unix::fs::symlink(target, path).unwrap();
        } else if let Some(length) = entry["zero_bytes"].as_u64() {
            assert!(length <= 128 << 20);
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .unwrap()
                .set_len(length)
                .unwrap();
        } else {
            std::fs::write(
                path,
                base64::engine::general_purpose::STANDARD
                    .decode(entry["body"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
        }
    }
    fixture
}

fn execute(request: &Value) -> Value {
    let fixture = std::env::var_os("D2RS_FIXTURE_ROOT").map(|_| materialize(request));
    let files = match request["policy"].as_str().unwrap() {
        "rooted" => ImageLocalFiles::rooted(Path::new(request["root"].as_str().unwrap())).unwrap(),
        "unrestricted" => ImageLocalFiles::Unrestricted,
        "denied" => ImageLocalFiles::default(),
        _ => panic!("unknown policy"),
    };
    let (output, error) = d2_rs_cli::imgbundler::bundle_local_with_policy(
        Path::new(request["input_path"].as_str().unwrap()),
        request["source"].as_str().unwrap().as_bytes(),
        files,
        request["cache"].as_bool().unwrap(),
        None,
    );
    if let Some(fixture) = fixture {
        std::fs::remove_dir_all(fixture).unwrap();
    }
    json!({"output":String::from_utf8(output).unwrap(),"error":error.unwrap_or_default()})
}
fn main() {
    let request: Value =
        serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let out = if let Value::Array(requests) = request {
        Value::Array(requests.iter().map(execute).collect())
    } else {
        execute(&request)
    };
    println!("{out}");
}
