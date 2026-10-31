// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

// These complete Go CLI artifacts exercise selected descendants, quoted and
// shorthand names, folder-only boards, and both process I/O boundaries.
// Generator: .reference/codex-runs/witness-inputs/run38-gif-subtree.py.
#[test]
fn gif_subtrees_and_stdio_match_original_go_frames() {
    let cases: serde_json::Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/gif-subtree-source-v090.json")).unwrap();
    let root = std::env::temp_dir().join(format!("d2-rs-gif-subtree-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let directory = root.join(name);
        std::fs::create_dir(&directory).unwrap();
        let input = directory.join("input.d2");
        let output = directory.join("out.gif");
        let source = case["source"].as_str().unwrap();
        std::fs::write(&input, source).unwrap();
        let use_stdin = case["stdin"] == true;
        let use_stdout = case["stdout"] == true;
        let mut command = Command::new(env!("CARGO_BIN_EXE_d2-rs"));
        command
            .current_dir(&directory)
            .args([
                "--pad=0",
                "--bundle=false",
                "--animate-interval=10",
                "--stdout-format=gif",
            ])
            .arg(format!("--target={}", case["target"].as_str().unwrap()))
            .arg(if use_stdin {
                std::path::Path::new("-")
            } else {
                &input
            })
            .arg(if use_stdout {
                std::path::Path::new("-")
            } else {
                &output
            })
            .stdin(if use_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut process = command.spawn().unwrap();
        if use_stdin {
            process
                .stdin
                .take()
                .unwrap()
                .write_all(source.as_bytes())
                .unwrap();
        }
        let result = process.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let actual = if use_stdout {
            assert!(!output.exists(), "{name}: stdout created an output file");
            result.stdout
        } else {
            assert!(
                result.stdout.is_empty(),
                "{name}: file output leaked to stdout"
            );
            std::fs::read(&output).unwrap()
        };
        assert_eq!(
            std::fs::read_dir(&directory).unwrap().count(),
            if use_stdout { 1 } else { 2 },
            "{name}: unexpected output"
        );
        let hex = case["gif_hex"].as_str().unwrap();
        let expected: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let decode = |bytes: &[u8]| {
            assert!(bytes.ends_with(b";"), "{name}: incomplete GIF");
            let mut options = gif::DecodeOptions::new();
            options.set_color_output(gif::ColorOutput::RGBA);
            let mut decoder = options.read_info(Cursor::new(bytes)).unwrap();
            let canvas = (decoder.width(), decoder.height());
            let repeat = decoder.repeat();
            let mut frames = Vec::new();
            while let Some(frame) = decoder.read_next_frame().unwrap() {
                assert_eq!(
                    (frame.left, frame.top, frame.width, frame.height),
                    (0, 0, canvas.0, canvas.1),
                    "{name}: full-canvas frame contract"
                );
                frames.push((frame.delay, frame.buffer.to_vec()));
            }
            assert!(!frames.is_empty(), "{name}: missing frames");
            (canvas, repeat, frames)
        };
        assert_eq!(decode(&actual), decode(&expected), "{name}");
    }
    std::fs::remove_dir_all(root).unwrap();
}
