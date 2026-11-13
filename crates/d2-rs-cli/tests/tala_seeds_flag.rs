// SPDX-License-Identifier: MPL-2.0
//! `--tala-seeds` follows pflag's Int64Slice (d2cli/layout_tala.go): the
//! first explicit occurrence replaces the default seeds, later occurrences
//! append, each value is split on commas and parsed with
//! `strconv.ParseInt(item, 0, 64)`, and no environment variable feeds it.
//! The equalities below were observed on the pinned Go D2 (6a7b4837) CLI.
use std::io::Write;
use std::process::{Command, Stdio};

// A nested diagram whose TALA result depends on the seed list: Go renders
// seeds [1, 2] differently from seeds [2].
const SOURCE: &str = "0{r.g-r.s.t\nr.s.u.v->r.z\nr.a->r.s.t\nr.s.t->g\nr.s.t->h}\n";

fn render(args: &[&str], environment: &[(&str, &str)]) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_d2-rs"));
    command
        .arg("--layout=tala")
        .args(args)
        .args(["-", "-"])
        .env_remove("D2_TALA_SEEDS")
        .env_remove("D2_TALA_RS_SEEDS");
    for (key, value) in environment {
        command.env(key, value);
    }
    let mut process = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    process
        .stdin
        .take()
        .unwrap()
        .write_all(SOURCE.as_bytes())
        .unwrap();
    let output = process.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn repeated_tala_seeds_flags_append() {
    let combined = render(&["--tala-seeds=1,2"], &[]);
    assert_ne!(
        combined,
        render(&["--tala-seeds=2"], &[]),
        "the witness must depend on the seed list"
    );
    assert_eq!(render(&["--tala-seeds=1", "--tala-seeds=2"], &[]), combined);
    assert_eq!(
        render(&["--tala-seeds", "1", "--tala-seeds", "2"], &[]),
        combined
    );
    assert_eq!(
        render(&["--tala-seeds", "1,2", "--tala-seeds", "3"], &[]),
        render(&["--tala-seeds=1,2,3"], &[])
    );
}

#[test]
fn tala_seeds_parse_with_go_base_prefixes() {
    // ParseInt(base 0): 0x2 and 0b10 are seed 2; 0o1 and 01 are seed 1.
    assert_eq!(
        render(&["--tala-seeds=0o1,0x2"], &[]),
        render(&["--tala-seeds=1,2"], &[])
    );
    assert_eq!(
        render(&["--tala-seeds=01", "--tala-seeds=0b10"], &[]),
        render(&["--tala-seeds=1,2"], &[])
    );
}

#[test]
fn tala_seeds_ignore_the_environment() {
    // registerTALAFlags passes an empty environment key to xmain.Int64Slice.
    let default = render(&[], &[]);
    assert_eq!(render(&[], &[("D2_TALA_SEEDS", "2")]), default);
    assert_eq!(
        render(&["--tala-seeds=1"], &[("D2_TALA_SEEDS", "2")]),
        render(&["--tala-seeds=1"], &[])
    );
}

#[test]
fn malformed_tala_seeds_report_the_pflag_error() {
    for (value, item) in [("", ""), ("1,,2", ""), ("a", "a")] {
        let output = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .args(["--layout=tala", &format!("--tala-seeds={value}"), "-", "-"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "err: bad usage: failed to parse flags: invalid argument \"{value}\" for \
                 \"--tala-seeds\" flag: strconv.ParseInt: parsing \"{item}\": invalid syntax\n\
                 err: Run with --help to see usage.\n"
            )
        );
    }
}
