// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn named_wildcards_write_complete_source_board_trees() {
    // Go d2cli.main's noChildren=false and renderWithPNGEncoder traversal,
    // including selected leaves, folder-only boards and mixed collections.
    // Generator: .reference/codex-runs/witness-inputs/run33-subtree-output.py.
    let cases: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/subtree-output-source-v090.json"))
    .unwrap();
    let root = std::env::temp_dir().join(format!("d2-rs-subtree-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for case in cases.as_array().unwrap() {
        let directory = root.join(case["name"].as_str().unwrap());
        std::fs::create_dir_all(&directory).unwrap();
        let input = directory.join("input.d2");
        let output = directory.join("out.svg");
        std::fs::write(&input, case["source"].as_str().unwrap()).unwrap();
        for split_flag in [false, true] {
            let mut args = vec![
                "--pad=0".into(),
                "--bundle=false".into(),
                "--omit-version".into(),
                "--no-xml-tag".into(),
            ];
            let target = case["target"].as_str().unwrap();
            if split_flag {
                args.extend(["--target".into(), target.into()]);
            } else {
                args.push(format!("--target={target}"));
            }
            args.extend([
                input.to_string_lossy().into_owned(),
                output.to_string_lossy().into_owned(),
            ]);
            assert!(run_document(&args).unwrap().is_empty());
            let mut pending = vec![directory.clone()];
            let mut actual = BTreeMap::new();
            while let Some(parent) = pending.pop() {
                for entry in std::fs::read_dir(parent).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        pending.push(path);
                    } else if path != input {
                        actual.insert(
                            path.strip_prefix(&directory)
                                .unwrap()
                                .to_string_lossy()
                                .into_owned(),
                            std::fs::read_to_string(path).unwrap(),
                        );
                    }
                }
            }
            let mut expected: BTreeMap<String, String> =
                serde_json::from_value(case["outputs"].clone()).unwrap();
            // Master (d2cli/board_output_publish.go) also records the tree it
            // published, at the output root, whenever the rendered subtree
            // has child boards.
            let manifest_name = format!("out/{}", crate::board_output::BOARD_OUTPUT_MANIFEST_NAME);
            if let Some(manifest) = actual.get(&manifest_name) {
                let listed = serde_json::from_str::<serde_json::Value>(manifest).unwrap()["files"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|file| format!("out/{}", file["path"].as_str().unwrap()))
                    .collect::<Vec<_>>();
                assert_eq!(listed, expected.keys().cloned().collect::<Vec<_>>(), "{target}");
                expected.insert(manifest_name.clone(), manifest.clone());
            }
            assert_eq!(
                actual.keys().collect::<Vec<_>>(),
                expected.keys().collect::<Vec<_>>(),
                "{target}"
            );
            for (name, expected) in &expected {
                assert_eq!(&actual[name], expected, "{target}: {name}");
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
