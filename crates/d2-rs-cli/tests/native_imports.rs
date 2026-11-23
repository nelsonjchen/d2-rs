// SPDX-License-Identifier: MPL-2.0
use std::{fs, path::Path, process::Command};

// Upstream 816c893b6, e2etests-cli/import_from_another_directory;
// nested traversal also exercises the owning file's path at each import hop.
fn check_imports(nested: bool) {
    let root = std::env::temp_dir().join(format!(
        "d2-rs-native-imports-{}-{nested}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("gallery/sub")).unwrap();
    let source = "tower -> receiver";
    fs::write(root.join("reference.d2"), source).unwrap();
    fs::write(root.join("tideglass.d2"), "unrelated").unwrap();
    fs::write(root.join("gallery/radio.d2"), "...@tideglass").unwrap();
    fs::write(
        root.join("gallery/tideglass.d2"),
        if nested { "...@sub/relay" } else { source },
    )
    .unwrap();
    fs::write(root.join("gallery/sub/relay.d2"), "...@../signal").unwrap();
    fs::write(root.join("gallery/signal.d2"), source).unwrap();
    let render = |input: &Path, output: &str| {
        let result = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .current_dir(&root)
            .args(["--isometric", "--scale=.2"])
            .arg(input)
            .arg(output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        fs::read(root.join(output)).unwrap()
    };
    let expected = render(Path::new("reference.d2"), "reference.gif");
    assert!(expected.starts_with(b"GIF89a"));
    for input in [
        Path::new("gallery/radio.d2").to_path_buf(),
        root.join("gallery/radio.d2"),
    ] {
        assert_eq!(render(&input, "radio.gif"), expected, "{}", input.display());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn imports_beside_the_importing_file() {
    check_imports(false);
}

#[test]
fn nested_imports_beside_each_importing_file() {
    check_imports(true);
}
