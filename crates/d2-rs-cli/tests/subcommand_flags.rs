// SPDX-License-Identifier: MPL-2.0
use std::{fs, path::Path, process::Command};

fn d2(directory: &Path, env: &[(&str, &str)], args: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_d2-rs"));
    command.current_dir(directory);
    for name in ["D2_SKETCH", "D2_THEME", "D2_ISOMETRIC", "D2_RS_NO_BROWSER"] {
        command.env_remove(name);
    }
    command.env("BROWSER", "0");
    for (name, value) in env {
        command.env(name, value);
    }
    command.args(args).output().unwrap()
}

#[test]
fn play_reads_theme_and_sketch_from_flags_or_environment() {
    // d2cli/play.go: GetInt64("theme")/GetBool("sketch") see explicit flags
    // first, then xmain's environment defaults; source config is not read.
    // The URL uses lib/urlenc.Encode and is logged at info level.
    let root = std::env::temp_dir().join(format!("d2-play-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("input.d2"), "a -> b\n").unwrap();
    let url = |sketch: u8, theme: i64| {
        format!(
            "info: opening playground: https://play.d2lang.com/?script=SlTQtVNI4gIMAA==&sketch={sketch}&theme={theme}&\n"
        )
    };
    for (env, args, expected) in [
        (&[][..], &["play", "input.d2"][..], url(0, 0)),
        (&[], &["--sketch", "play", "input.d2"], url(1, 0)),
        (&[], &["play", "-s", "-t", "3", "input.d2"], url(1, 3)),
        (
            &[("D2_SKETCH", "true"), ("D2_THEME", "4")],
            &["play", "input.d2"],
            url(1, 4),
        ),
        (
            &[("D2_SKETCH", "true")],
            &["--sketch=false", "play", "input.d2"],
            url(0, 0),
        ),
        (
            &[],
            &["--pad=5", "--layout=elk", "play", "input.d2"],
            url(0, 0),
        ),
    ] {
        let result = d2(&root, env, args);
        assert!(result.status.success(), "{args:?}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stderr).unwrap(),
            expected,
            "{args:?}"
        );
    }
    let result = d2(&root, &[], &["--isometric", "play", "input.d2"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("isometric images are local exports; use an output filename instead of play")
    );
    // xbrowser.Open runs `sh -sc '<BROWSER> "$1"'`; a failure only warns.
    // A function keeps the URL argument away from `exit`: bash (macOS's sh)
    // rejects `exit 3 <url>` as too many arguments, dash ignores the extra.
    let result = d2(
        &root,
        &[("BROWSER", "fail() { exit 3; }; fail")],
        &["play", "input.d2"],
    );
    assert!(result.status.success(), "{result:?}");
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(stderr.contains("warn: failed to open browser to https://play.d2lang.com/"));
    assert!(stderr.ends_with("(out: \"\"): exit status 3\n"), "{stderr}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn subcommands_dispatch_on_the_first_positional_after_flags() {
    // d2cli.Run switches on ms.Opts.Flags.Arg(0) after pflag removed flags.
    let root = std::env::temp_dir().join(format!("d2-subcommand-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("input.d2"), "a -> b\n").unwrap();
    for args in [
        &["--sketch", "validate", "input.d2"][..],
        &["--pad=5", "fmt", "input.d2"],
        &["--theme=3", "themes"],
        &["-s", "layout", "elk"],
        &["layout", "--sketch", "elk"],
        &["--isometric", "layout"],
        &["--center", "version"],
    ] {
        let result = d2(&root, &[], args);
        assert!(result.status.success(), "{args:?}: {result:?}");
    }
    let plain = d2(&root, &[], &["layout", "elk"]).stdout;
    assert_eq!(d2(&root, &[], &["--sketch", "layout", "elk"]).stdout, plain);
    fs::remove_dir_all(root).unwrap();
}
