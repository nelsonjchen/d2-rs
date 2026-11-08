// SPDX-License-Identifier: MPL-2.0
use std::{collections::BTreeMap, fs, path::Path, process::Command};

fn inventory(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    visit(root, root, &mut files);
    files
}

#[test]
fn mapless_sibling_boards_preserve_complete_output_and_recursive_hashes() {
    // Pinned d2compiler/compile.go compileBoardsField uses an empty map for
    // mapless fields. d2target.Diagram.Bytes includes that empty child too.
    // Both spellings produce identical complete inventories in pinned Go.
    let root = std::env::temp_dir().join(format!("d2-mapless-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    for engine in ["dagre", "elk", "tala"] {
        let mut outputs = Vec::new();
        for (variant, source) in [
            ("mapless", "e\nlayers{w{s{1{s{}}\n2{}}}\nr}"),
            ("mapped", "e\nlayers{w{s{1{s{}}\n2{}}}\nr{}}"),
        ] {
            let directory = root.join(format!("{engine}-{variant}"));
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("input.d2"), source).unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
                .current_dir(&directory)
                .args([&format!("--layout={engine}"), "input.d2", "output.svg"])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let files = inventory(&directory.join("output"));
            assert!(files.contains_key("index.svg"));
            assert!(files.contains_key("w.svg"));
            assert!(!files.contains_key("r.svg"));
            outputs.push(files);
        }
        assert_eq!(outputs[0], outputs[1], "{engine}");
    }
    fs::remove_dir_all(root).unwrap();
}
