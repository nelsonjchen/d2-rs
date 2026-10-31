// SPDX-License-Identifier: MPL-2.0
//! Input-only transport for original canceled-format/empty-layout calls.
use super::*;
use base64::Engine as _;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;

#[test]
#[ignore = "executed explicitly by cli-admission-scoreboard.py"]
fn replay() {
    let requests: Vec<serde_json::Value> = serde_json::from_slice(
        &std::fs::read(std::env::var_os("D2RS_ADMISSION_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let initial_directory = std::env::current_dir().unwrap();
    let mut results = Vec::new();
    for input in requests {
        let directory = Path::new(input["directory"].as_str().unwrap());
        for (name, file) in input["files"].as_object().unwrap() {
            let path = directory.join(name);
            if file.is_null() {
                if path.exists() {
                    std::fs::remove_file(path).unwrap();
                }
            } else {
                let data = base64::engine::general_purpose::STANDARD
                    .decode(file["data"].as_str().unwrap())
                    .unwrap();
                std::fs::write(&path, data).unwrap();
                std::fs::set_permissions(
                    &path,
                    std::fs::Permissions::from_mode(file["mode"].as_u64().unwrap() as u32),
                )
                .unwrap();
            }
        }
        std::env::set_current_dir(directory).unwrap();
        let token = d2_rs_core::CancellationToken::new();
        if input["canceled"].as_bool().unwrap() {
            token.cancel();
        }
        let _guard = watch::track_cancellation(token);
        let args: Vec<String> = serde_json::from_value(input["args"].clone()).unwrap();
        let (stdout, error) = match dispatch(&args) {
            Ok(bytes) => (bytes, String::new()),
            Err(error) => {
                let mut terminal = Vec::new();
                write_cli_error(&mut terminal, error.terminal_message()).unwrap();
                (Vec::new(), String::from_utf8(terminal).unwrap())
            }
        };
        let mut files = serde_json::Map::new();
        let mut names: BTreeSet<String> = input["files"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        names.extend(
            std::fs::read_dir(directory)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap()),
        );
        for name in &names {
            let path = directory.join(name);
            let file = match std::fs::read(&path) {
                Ok(bytes) => {
                    json!({"data":base64::engine::general_purpose::STANDARD.encode(bytes),"mode":std::fs::metadata(path).unwrap().permissions().mode() & 0o777})
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    serde_json::Value::Null
                }
                Err(error) => panic!("{error}"),
            };
            files.insert(name.clone(), file);
        }
        results.push(json!({"error":error,"stdout":base64::engine::general_purpose::STANDARD.encode(stdout),"files":files}));
        std::env::set_current_dir(&initial_directory).unwrap();
    }
    std::fs::write(
        std::env::var_os("D2RS_ADMISSION_RESPONSE").unwrap(),
        serde_json::to_vec(&results).unwrap(),
    )
    .unwrap();
}

#[test]
fn canceled_formatter_preserves_file_bytes_and_mode() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("fmt-cancellation-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("input.d2");
    for expired in [false, true] {
        let input = b"x:{y}\n";
        std::fs::write(&path, input).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let token = d2_rs_core::CancellationToken::new();
        if expired {
            token.expire();
        } else {
            token.cancel();
        }
        let _guard = watch::track_cancellation(token);
        let error = run_fmt(&[path.display().to_string()]).unwrap_err();
        assert_eq!(
            error.to_string(),
            if expired {
                "failed to fmt: context deadline exceeded"
            } else {
                "failed to fmt: context canceled"
            }
        );
        assert_eq!(std::fs::read(&path).unwrap(), input);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}
