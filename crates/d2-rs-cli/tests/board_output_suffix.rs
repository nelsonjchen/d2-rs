// SPDX-License-Identifier: MPL-2.0
use std::{fs, path::Path, process::Command};

fn run(directory: &Path, flags: &[&str], output: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .current_dir(directory)
        .args(["--layout=dagre", "--omit-version"])
        .args(flags)
        .args(["input.d2", output])
        .output()
        .unwrap()
}

#[test]
fn recursive_outputs_and_board_links_keep_the_requested_suffix() {
    // Pinned d2cli/board_output.go splitBoardOutputPath and
    // appendBoardOutputComponent carry filepath.Ext, not the encoder name.
    let root = std::env::temp_dir().join(format!("d2-suffix-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    for (index, (output, directory, suffix)) in [
        ("out.SVG", "out", ".SVG"),
        ("out.bad", "out", ".bad"),
        ("out", "out", ""),
        ("out.", "out", "."),
        (".SVG", ".SVG", ".SVG"),
    ]
    .into_iter()
    .enumerate()
    {
        let work = root.join(index.to_string());
        fs::create_dir_all(&work).unwrap();
        fs::write(
            work.join("input.d2"),
            "a: {link: root.scenarios.review}\n\
             q: {link: root.layers.\"one.two\"}\n\
             layers: {\"one.two\": {b: {link: root}}}\n\
             scenarios: {review: {c}}\n",
        )
        .unwrap();
        let result = run(&work, &[], output);
        assert!(result.status.success(), "{:?}", result);
        let tree = work.join(directory);
        let write_suffix = if suffix.is_empty() {
            let file = fs::read_dir(&tree)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .find(|name| name.to_string_lossy().starts_with("index.d2-board-output-"))
                .unwrap();
            file.to_str()
                .unwrap()
                .strip_prefix("index")
                .unwrap()
                .to_owned()
        } else {
            suffix.to_owned()
        };
        let root_svg = fs::read_to_string(tree.join(format!("index{write_suffix}"))).unwrap();
        let child_svg =
            fs::read_to_string(tree.join(format!("layers/one.two{write_suffix}"))).unwrap();
        assert!(root_svg.contains(&format!("href=\"scenarios/review{suffix}\"")));
        assert!(root_svg.contains("href=\"root.layers.&#34;one.two&#34;\""));
        assert!(child_svg.contains(&format!("href=\"../index{suffix}\"")));
        assert!(
            tree.join(format!("scenarios/review{write_suffix}"))
                .is_file()
        );
        assert!(tree.join(".d2-board-output-manifest-v1.json").is_file());

        // A selected leaf still resolves links against the full document.
        let result = run(&work, &["--target=\"one.two\""], &format!("leaf{suffix}"));
        assert!(result.status.success(), "{:?}", result);
        let leaf = fs::read_to_string(work.join(format!("leaf{suffix}"))).unwrap();
        assert!(leaf.contains(&format!("href=\"../index{suffix}\"")));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn txt_named_wildcards_use_recursive_paths_and_preserve_write_errors() {
    // main.go renderWithPNGEncoderToPaths visits named TXT targets too;
    // its TXT writeWithStatus branch does not create missing directories.
    let root = std::env::temp_dir().join(format!("d2-txt-target-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("input.d2"), "a; layers: {one: {b}}\n").unwrap();
    let result = run(&root, &["--target=one.*"], "out.txt");
    assert!(!result.status.success());
    let error = String::from_utf8(result.stderr).unwrap();
    assert!(error.contains("one.txt"), "{error}");
    assert!(!root.join("out.txt").exists());
    assert!(!root.join("out").exists());

    fs::create_dir(root.join("out")).unwrap();
    let result = run(&root, &["--target=one.*"], "out.txt");
    assert!(result.status.success(), "{:?}", result);
    let recursive = fs::read(root.join("out/one.txt")).unwrap();
    let result = run(&root, &["--target=one"], "single.txt");
    assert!(result.status.success(), "{:?}", result);
    assert_eq!(recursive, fs::read(root.join("single.txt")).unwrap());
    assert!(!root.join("out/index.txt").exists());
    fs::remove_dir_all(root).unwrap();
}
