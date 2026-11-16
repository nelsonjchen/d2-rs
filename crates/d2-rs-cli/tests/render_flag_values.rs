// SPDX-License-Identifier: MPL-2.0
use std::{fs, path::Path, process::Command};

fn render(directory: &Path, flags: &[&str], output: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .current_dir(directory)
        .env_remove("D2_DARK_THEME")
        .args(["--layout=dagre", "--omit-version"])
        .args(flags)
        .args(["input.d2", output])
        .output()
        .unwrap()
}

#[test]
fn render_ignores_values_that_only_other_modes_consume() {
    // Pinned d2cli.Run: --dark-theme -1 means unset (main.go darkThemeFlag
    // sentinel), --port is an unvalidated string outside watch mode, and
    // --check applies only to `fmt`.
    let root = std::env::temp_dir().join(format!("d2-flag-values-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("input.d2"), "a -> b\n").unwrap();
    let plain = render(&root, &[], "plain.svg");
    assert!(plain.status.success(), "{plain:?}");
    let expected = fs::read(root.join("plain.svg")).unwrap();
    for (index, flags) in [
        &["--dark-theme=200", "--dark-theme=-1"][..],
        &["--dark-theme", "-1"],
        &["--port=abc"],
        &["--check=false"],
        &["--check"],
        &["--debug=false"],
    ]
    .into_iter()
    .enumerate()
    {
        let output = format!("out{index}.svg");
        let result = render(&root, flags, &output);
        assert!(result.status.success(), "{flags:?}: {result:?}");
        assert_eq!(fs::read(root.join(&output)).unwrap(), expected, "{flags:?}");
    }
    let result = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .current_dir(&root)
        .env("D2_DARK_THEME", "-1")
        .args(["--layout=dagre", "--omit-version", "input.d2", "env.svg"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(fs::read(root.join("env.svg")).unwrap(), expected);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn colliding_joined_board_names_link_to_the_last_board_in_preorder() {
    // main.go resolveLinks merges child maps in board preorder, so a literal
    // layer "x.layers.y" and the nested x -> y share one key and the later
    // board wins; relink then compares that raw key.
    let root = std::env::temp_dir().join(format!("d2-link-collide-{}", std::process::id()));
    for (index, (layers, href)) in [
        (r#"{"x.layers.y": {a}; x: {layers: {y: {b}}}}"#, "x/y.svg"),
        (
            r#"{x: {layers: {y: {b}}}; "x.layers.y": {a}}"#,
            "x.layers.y.svg",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let work = root.join(index.to_string());
        fs::create_dir_all(&work).unwrap();
        fs::write(
            work.join("input.d2"),
            format!("z: {{link: root.layers.x.layers.y}}\nlayers: {layers}\n"),
        )
        .unwrap();
        let result = render(&work, &[], "out.svg");
        assert!(result.status.success(), "{result:?}");
        let index_svg = fs::read_to_string(work.join("out/index.svg")).unwrap();
        assert!(index_svg.contains(&format!("href=\"{href}\"")), "{layers}");
    }
    fs::remove_dir_all(root).unwrap();
}

fn render_env(directory: &Path, env: &[(&str, &str)], flags: &[&str], output: &str) -> Vec<u8> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_d2-rs"));
    command.current_dir(directory);
    for name in ["D2_SKETCH", "D2_ANIMATE_INTERVAL", "D2_CENTER"] {
        command.env_remove(name);
    }
    for (name, value) in env {
        command.env(name, value);
    }
    let result = command
        .args(["--layout=dagre", "--omit-version"])
        .args(flags)
        .args(["input.d2", output])
        .output()
        .unwrap();
    assert!(result.status.success(), "{env:?} {flags:?}: {result:?}");
    fs::read(directory.join(output)).unwrap()
}

#[test]
fn explicit_and_environment_sketch_values_precede_source_config() {
    // main.go keeps sketchFlag when D2_SKETCH is non-empty or the flag was
    // set; d2lib applyConfigs only fills a nil Sketch. xmain.getEnv treats an
    // empty value as unset.
    let root = std::env::temp_dir().join(format!("d2-sketch-order-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("input.d2"), "a -> b\n").unwrap();
    let plain = render_env(&root, &[], &[], "plain.svg");
    let sketched = render_env(&root, &[], &["--sketch"], "sketched.svg");
    assert_ne!(plain, sketched);
    fs::write(
        root.join("input.d2"),
        "vars: {d2-config: {sketch: true}}\na -> b\n",
    )
    .unwrap();
    let configured_plain = render_env(&root, &[], &["--sketch=false"], "flag.svg");
    assert_eq!(
        configured_plain,
        render_env(&root, &[("D2_SKETCH", "false")], &[], "env.svg")
    );
    assert_eq!(
        configured_plain,
        render_env(
            &root,
            &[("D2_SKETCH", "true")],
            &["--sketch=false"],
            "both.svg"
        )
    );
    assert_ne!(configured_plain, render_env(&root, &[], &[], "config.svg"));
    assert_eq!(
        render_env(&root, &[("D2_SKETCH", "")], &[], "empty.svg"),
        render_env(&root, &[], &[], "config2.svg")
    );
    // A zero D2_ANIMATE_INTERVAL is the Int64 default: no animation.
    fs::write(root.join("input.d2"), "a\nsteps: {one: {b}}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .current_dir(&root)
        .env("D2_ANIMATE_INTERVAL", "0")
        .args(["--layout=dagre", "input.d2", "steps.svg"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(root.join("steps/one.svg").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn paged_exports_reject_duplicate_board_ids() {
    // d2cli/paged.go indexPagedBoards, used by PDF, PPTX and isometric paged
    // exports; GIF and SVG board trees have no such check.
    let root = std::env::temp_dir().join(format!("d2-paged-dup-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("input.d2"),
        "layers: {\"x.layers.y\": {a}; x: {layers: {y: {b}}}}\n",
    )
    .unwrap();
    for (flags, output) in [
        (&[][..], "out.pdf"),
        (&[], "out.pptx"),
        (&["--isometric"], "iso.pdf"),
    ] {
        let result = render(&root, flags, output);
        assert_eq!(result.status.code(), Some(1), "{result:?}");
        let error = String::from_utf8(result.stderr).unwrap();
        assert!(
            error.contains(
                "failed to compile input.d2: paged board tree contains duplicate board ID \"root.layers.x.layers.y\""
            ),
            "{error}"
        );
        assert!(!root.join(output).exists());
    }
    fs::remove_dir_all(root).unwrap();
}
