// SPDX-License-Identifier: MPL-2.0
//! `--debug` / `$DEBUG` logging contracts. Expected records are the pinned Go
//! CLI's output (d2cli/main.go ms.Log.Debug lines on stderr; lib/log
//! PrettyHandler lines from d2layouts.LayoutNested on stdout). The slog
//! timestamp is the only run-varying field; SOURCE_DATE_EPOCH pins it here.
use std::{fs, path::PathBuf, process::Command};

const TS: &str = "\x1b[2m2000-01-02T03:04:05Z\x1b[0m \x1b[36mDEBUG\x1b[0m ";

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn workspace(name: &str, source: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("d2-debug-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("input.d2"), source).unwrap();
    root
}

fn run(root: &PathBuf, env: &[(&str, &str)], args: &[&str]) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_d2-rs"));
    command
        .current_dir(root)
        .env("SOURCE_DATE_EPOCH", "946782245")
        .env("TZ", "UTC")
        .env_remove("DEBUG")
        .env_remove("D2_LAYOUT")
        .env_remove("D2_THEME")
        .env_remove("D2_DARK_THEME")
        .env_remove("D2_RS_CLI_PROCESS_LOG");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.args(args).output().unwrap();
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn records(lines: &[&str]) -> String {
    lines.iter().map(|line| format!("{TS}{line}\n")).collect()
}

/// stderr without the run-varying elapsed time of the `success:` line.
fn stderr_lines(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .map(|line| match line.rsplit_once(" in ") {
            Some((head, _)) if line.contains("success: ") => format!("{head} in T"),
            _ => line.to_owned(),
        })
        .collect()
}

#[test]
fn nested_layout_records_follow_layout_nested_object_moves() {
    // Grid inside nested containers, with a sequence diagram inside a grid
    // cell: extraction, grid-cell re-extraction and injection order.
    let root = workspace(
        "deep",
        "a: {b: {c: {grid-columns: 1; d: {e: {shape: sequence_diagram; f -> g}}; h}}}\ni -> a.b\n",
    );
    for layout in ["dagre", "elk", "tala"] {
        let result = run(
            &root,
            &[],
            &["--debug", "-l", layout, "input.d2", "out.svg"],
        );
        assert_eq!(result.code, 0, "{}", result.stderr);
        assert_eq!(
            result.stdout,
            records(&[
                "layout nested level=3 child=a.b.c gi={false grid-diagram}",
                "layout nested level=5 child=d.e gi={false sequence-diagram}",
                "layout sequence rootlevel=5 shapes=Objects: [f, g, ]",
                "done rootlevel=5 shapes=Objects: [f, g, ]",
                "default layout rootlevel=3 shapes=Objects: [d, d.e, ]",
                "done rootlevel=3 shapes=Objects: [d, d.e, d.e.f, d.e.g, ]",
                "layout grid rootlevel=3 shapes=Objects: [d, h, ]",
                "done rootlevel=3 shapes=Objects: [d, h, d.e, d.e.f, d.e.g, ]",
                "default layout rootlevel=0 shapes=Objects: [a, a.b, a.b.c, i, ]",
                "done rootlevel=0 shapes=Objects: [a, a.b, a.b.c, i, a.b.c.d, a.b.c.d.e, a.b.c.d.e.f, a.b.c.d.e.g, a.b.c.h, ]",
            ]),
            "{layout}"
        );
        assert_eq!(
            stderr_lines(&result.stderr),
            [
                "debug: using theme Neutral Default (ID: 0)".to_owned(),
                format!("debug: using layout engine {layout} (built-in)"),
                "success: successfully compiled input.d2 to out.svg in T".to_owned(),
            ]
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn root_grid_and_constant_near_records_match_go() {
    let root = workspace(
        "grid",
        "grid-rows: 2\na: {x -> y}\nb\nc: {grid-columns: 1; p; q}\n",
    );
    let result = run(&root, &[], &["--debug", "input.d2", "out.svg"]);
    assert_eq!(
        result.stdout,
        records(&[
            "default layout rootlevel=0 shapes=Objects: [a, a.x, a.y, ]",
            "done rootlevel=0 shapes=Objects: [a, a.x, a.y, ]",
            "layout nested level=1 child=c gi={false grid-diagram}",
            "layout grid rootlevel=1 shapes=Objects: [p, q, ]",
            "done rootlevel=1 shapes=Objects: [p, q, ]",
            "layout grid rootlevel=0 shapes=Objects: [a, b, c, ]",
            "done rootlevel=0 shapes=Objects: [a, b, c, a.x, a.y, c.p, c.q, ]",
        ])
    );
    fs::write(
        root.join("input.d2"),
        "a -> b\nt: {near: top-center; label: title}\nk: {near: bottom-right; m -> n}\n",
    )
    .unwrap();
    let result = run(&root, &[], &["--debug", "input.d2", "out.svg"]);
    assert_eq!(
        result.stdout,
        records(&[
            "layout nested level=1 child=t gi={true }",
            "default layout rootlevel=0 shapes=Objects: [t, ]",
            "done rootlevel=0 shapes=Objects: [t, ]",
            "layout nested level=1 child=k gi={true }",
            "default layout rootlevel=0 shapes=Objects: [k, k.m, k.n, ]",
            "done rootlevel=0 shapes=Objects: [k, k.m, k.n, ]",
            "default layout rootlevel=0 shapes=Objects: [a, b, ]",
            "done rootlevel=0 shapes=Objects: [a, b, t, k, k.m, k.n, ]",
        ])
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reused_scenario_and_step_layouts_are_not_logged_for_dagre_and_elk() {
    // d2lib compileBoard reuses the base geometry when a scenario or step
    // changes only render attributes; TALA never reuses.
    let root = workspace(
        "boards",
        "a -> b\nscenarios: {same: {a.style.fill: red}; add: {c}}\nsteps: {one: {b.style.fill: blue}; two: {d}; three: {d.style.fill: red}}\nlayers: {l: {q}}\n",
    );
    let base = [
        "default layout rootlevel=0 shapes=Objects: [a, b, ]",
        "done rootlevel=0 shapes=Objects: [a, b, ]",
    ];
    let layer = [
        "default layout rootlevel=0 shapes=Objects: [q, ]",
        "done rootlevel=0 shapes=Objects: [q, ]",
    ];
    let add = [
        "default layout rootlevel=0 shapes=Objects: [a, b, c, ]",
        "done rootlevel=0 shapes=Objects: [a, b, c, ]",
    ];
    let two = [
        "default layout rootlevel=0 shapes=Objects: [a, b, d, ]",
        "done rootlevel=0 shapes=Objects: [a, b, d, ]",
    ];
    for layout in ["dagre", "elk"] {
        let result = run(
            &root,
            &[],
            &["--debug", "-l", layout, "input.d2", "out.svg"],
        );
        assert_eq!(result.code, 0, "{}", result.stderr);
        let expected = [&base[..], &layer, &add, &two].concat();
        assert_eq!(result.stdout, records(&expected), "{layout}");
    }
    let result = run(
        &root,
        &[],
        &["--debug", "-l", "tala", "input.d2", "out.svg"],
    );
    let expected = [&base[..], &layer, &base, &add, &base, &two, &two].concat();
    assert_eq!(result.stdout, records(&expected));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn command_debug_lines_follow_the_debug_environment_at_write_time() {
    let root = workspace("env", "a -> b\n");
    // $DEBUG=1 prints ms.Log.Debug even with --debug=false, but the slog
    // context stays at Info.
    let result = run(
        &root,
        &[("DEBUG", "1")],
        &["--debug=false", "input.d2", "out.svg"],
    );
    assert_eq!(result.stdout, "");
    assert_eq!(
        stderr_lines(&result.stderr),
        [
            "debug: using theme Neutral Default (ID: 0)",
            "debug: using layout engine dagre (built-in)",
            "success: successfully compiled input.d2 to out.svg in T",
        ]
    );
    // An invalid $DEBUG is ignored by the flag and reported by every
    // Env.Debug check, without a newline.
    let result = run(&root, &[("DEBUG", "yes")], &["input.d2", "out.svg"]);
    assert_eq!(result.stdout, "");
    let fragment = "env: $DEBUG must be 0, 1, false or true but got \"yes\"";
    assert_eq!(
        stderr_lines(&result.stderr),
        [
            "warn: Invalid DEBUG flag value ignored".to_owned(),
            format!("{fragment}{fragment}success: successfully compiled input.d2 to out.svg in T"),
        ]
    );
    let result = run(&root, &[("DEBUG", "0")], &["input.d2", "out.svg"]);
    assert_eq!(
        stderr_lines(&result.stderr),
        ["success: successfully compiled input.d2 to out.svg in T"]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ascii_gif_and_theme_debug_lines_match_d2cli() {
    let root = workspace("formats", "a -> b\n");
    let result = run(&root, &[], &["--debug", "input.d2", "out.txt"]);
    assert_eq!(
        stderr_lines(&result.stderr),
        [
            "debug: using theme Neutral Default (ID: 0)",
            "debug: switching layout engine to ELK for ASCII format (was unspecified)",
            "debug: using layout engine elk (built-in)",
            "success: successfully compiled input.d2 to out.txt in T",
        ]
    );
    let result = run(
        &root,
        &[],
        &[
            "--debug",
            "--layout=Dagre",
            "-t",
            "300",
            "input.d2",
            "out.txt",
        ],
    );
    // Only the exact "dagre" flag spelling is substituted.
    assert_eq!(
        stderr_lines(&result.stderr)[..2],
        [
            "debug: using theme Terminal (ID: 300)",
            "debug: using layout engine dagre (built-in)",
        ]
    );
    let result = run(
        &root,
        &[],
        &["--debug", "--dark-theme=200", "input.d2", "out.gif"],
    );
    assert_eq!(
        stderr_lines(&result.stderr),
        [
            "debug: using theme Neutral Default (ID: 0)",
            "debug: using dark theme Dark Mauve (ID: 200)",
            "warn: --dark-theme cannot be used while exporting to another format other than .svg",
            "debug: GIF export: animate-interval not specified, defaulting to 1000ms",
            "debug: using layout engine dagre (built-in)",
            "success: successfully compiled input.d2 to out.gif in T",
        ]
    );
    // The slog records precede the document on a stdout output.
    let result = run(&root, &[], &["--debug", "input.d2", "-"]);
    assert!(
        result.stdout.starts_with(&records(&[
            "default layout rootlevel=0 shapes=Objects: [a, b, ]",
            "done rootlevel=0 shapes=Objects: [a, b, ]",
        ])),
        "{}",
        &result.stdout[..200.min(result.stdout.len())]
    );
    assert!(result.stdout.contains("<svg"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_failing_sequence_layout_ends_the_records_like_go_compile() {
    // d2sequence.newSequenceDiagram rejects a nested sequence actor after
    // LayoutNested logged "layout sequence"; d2cli then logs no engine line.
    let root = workspace(
        "seqerr",
        "shape: sequence_diagram\na: {shape: sequence_diagram; b -> c}\na -> d\n",
    );
    let result = run(&root, &[], &["--debug", "input.d2", "out.svg"]);
    assert_eq!(result.code, 1);
    assert_eq!(
        result.stdout,
        records(&[
            "layout nested level=1 child=a gi={false sequence-diagram}",
            "layout sequence rootlevel=1 shapes=Objects: [b, c, ]",
            "done rootlevel=1 shapes=Objects: [b, c, ]",
            "layout sequence rootlevel=0 shapes=Objects: [a, d, ]",
        ])
    );
    assert_eq!(
        stderr_lines(&result.stderr),
        [
            "debug: using theme Neutral Default (ID: 0)",
            "err: failed to compile input.d2: actors in sequence diagrams cannot themselves be sequence diagrams: a",
        ]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn layout_reuse_compares_go_signature_fields_not_label_text() {
    // layout_reuse.go writeAttributes records whether a label is set and its
    // dimensions, not its text: "21" measures like "12" and is reused.
    let root = workspace(
        "edgelabel",
        "a -> b: 12\nscenarios: {same: {(a -> b)[0].label: 21}; wider: {(a -> b)[0].label: 123}}\n",
    );
    for layout in ["dagre", "elk"] {
        let result = run(
            &root,
            &[],
            &["--debug", "-l", layout, "input.d2", "out.svg"],
        );
        assert_eq!(result.code, 0, "{}", result.stderr);
        assert_eq!(
            result.stdout,
            records(&[
                "default layout rootlevel=0 shapes=Objects: [a, b, ]",
                "done rootlevel=0 shapes=Objects: [a, b, ]",
                "default layout rootlevel=0 shapes=Objects: [a, b, ]",
                "done rootlevel=0 shapes=Objects: [a, b, ]",
            ]),
            "{layout}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_failing_tala_layout_ends_the_records_after_its_default_layout() {
    // d2talalayout validates the extracted grid cell before laying it out.
    let root = workspace(
        "talaerr",
        "g: {grid-rows: 1; c: {k: {width: 2000000000}}}\n",
    );
    let result = run(
        &root,
        &[],
        &["--debug", "-l", "tala", "input.d2", "out.svg"],
    );
    assert_eq!(result.code, 1);
    assert_eq!(
        result.stdout,
        records(&[
            "layout nested level=1 child=g gi={false grid-diagram}",
            "default layout rootlevel=1 shapes=Objects: [c, c.k, ]",
        ])
    );
    assert_eq!(
        stderr_lines(&result.stderr),
        [
            "debug: using theme Neutral Default (ID: 0)",
            "err: failed to compile input.d2: D2 object \"k\": width is outside the supported range: 2e+09",
        ]
    );
    fs::remove_dir_all(root).unwrap();
}
