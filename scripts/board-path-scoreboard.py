#!/usr/bin/env python3
"""Original CLI board name, collision and append-path containment observations.

No runtime output normalization. Native requests contain original inputs only.
Names travel as raw bytes, including invalid UTF-8 Go strings. Historical
unsupported responses remain non-passing when rescoring older executables.
"""
import argparse
import collections
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
TESTS = {
    "TestBoardOutputComponent": 62,
    "TestValidateBoardOutputPathsRejectsCollisions": 2,
    "TestValidateBoardOutputPathsRejectsFileDirectoryPrefixCollision": 2,
    "TestAppendBoardOutputNameStaysContained": 9,
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate_calls(calls):
    assert collections.Counter(c["test"].split("/")[0] for c in calls) == TESTS, "incomplete original calls"
    assert collections.Counter(c["test"] for c in calls) == {
        "TestBoardOutputComponent": 62,
        "TestValidateBoardOutputPathsRejectsCollisions/case_insensitive_filesystem": 1,
        "TestValidateBoardOutputPathsRejectsCollisions/same_escaped_component": 1,
        "TestValidateBoardOutputPathsRejectsFileDirectoryPrefixCollision": 2,
        "TestAppendBoardOutputNameStaysContained": 9,
    }, "changed original test sessions"
    for c in calls:
        if c["test"] == "TestAppendBoardOutputNameStaysContained":
            assert c["input"]["op"] in {"append_name", "append_component", "within"}
        else:
            assert c["input"]["op"] == ("component" if c["test"] == "TestBoardOutputComponent" else "validate")
    assert collections.Counter(c["input"]["op"] for c in calls
                               if c["test"] == "TestAppendBoardOutputNameStaysContained") == {
        "append_name": 7, "append_component": 1, "within": 1}, "incomplete containment calls"


def compare(calls, results):
    validate_calls(calls)
    assert len(results) == len(calls), "missing/extra native responses"
    rows = []
    for test, count in TESTS.items():
        indices = [i for i, c in enumerate(calls) if c["test"].split("/")[0] == test]
        matched = sum(results[i] == calls[i]["result"] for i in indices)
        unsupported = sum(results[i] == {"unsupported": "native board names are UTF-8 strings"}
                          and calls[i]["input"]["op"] == "component"
                          and not valid_utf8(calls[i]["input"]["bytes"]) for i in indices)
        rows.append(dict(test=test, calls=count, matched_calls=matched, unsupported_calls=unsupported,
                         outcome="match" if matched == count else "unsupported" if matched + unsupported == count else "mismatch"))
    return rows


def valid_utf8(values):
    try:
        bytes(values).decode("utf-8")
        return True
    except UnicodeDecodeError:
        return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-test", required=True, type=Path)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--reuse", action="store_true")
    args = parser.parse_args()
    source = references.verify_source()
    capture, output, runner = args.capture.resolve(), args.output.resolve(), args.native_test.resolve()
    spec = importlib.util.spec_from_file_location("cli", ROOT / "scripts/cli-scoreboard.py")
    cli = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(cli)
    original = (source / "d2cli/board_output_test.go").read_bytes()
    hook = ROOT / "tools/board-path-scoreboard/hook_test.go"
    binding = dict(source_commit=references.SOURCE_COMMIT, package="d2cli", hook_sha256=sha(hook),
                   test_sha256=hashlib.sha256(original).hexdigest(),
                   implementation_sha256=sha(source / "d2cli/board_output.go"))
    env = dict(os.environ, GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
               GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off")
    if not args.reuse:
        capture.mkdir(exist_ok=False)
        (capture / "tmp").mkdir()
        target = references.TRACE_CHECKOUT / "d2cli/board_output_test.go"
        helper = target.parent / "d2rs_board_paths_test.go"
        assert target.read_bytes() == original and not helper.exists(), "concurrent source mutation"
        text = original.decode()
        assert text.count("got := boardOutputComponent(name)") == 2
        assert text.count("got != boardOutputComponent(name)") == 1
        assert text.count("err := validateBoardOutputPaths(") == 2
        text = text.replace("got := boardOutputComponent(name)", "got := d2rsPathComponent(t, name)")
        text = text.replace("got != boardOutputComponent(name)", "got != d2rsPathComponent(t, name)")
        text = text.replace("err := validateBoardOutputPaths(", "err := d2rsPathValidate(t, ")
        for name, wrapper in [("appendBoardOutputName", "d2rsPathAppendName"),
                              ("appendBoardOutputComponent", "d2rsPathAppendComponent"),
                              ("ensurePathWithin", "d2rsPathWithin")]:
            # Only replace calls, preserving original assertion messages.
            assert text.count(":= " + name + "(") == 1
            text = text.replace(":= " + name + "(", ":= " + wrapper + "(t, ")
        env.update(TMPDIR=str(capture / "tmp"), D2RS_PATH_CAPTURE=str(capture))
        try:
            target.write_text(text)
            helper.write_bytes(hook.read_bytes())
            go = cli.bounded_run(["go", "test", "./d2cli", "-run", "^(" + "|".join(TESTS) + ")$",
                                  "-json", "-count=1", "-parallel=1"], env, capture, capture / "go.jsonl", 600)
        finally:
            target.write_bytes(original)
            helper.unlink(missing_ok=True)
        assert go == {"exit": 0, "failure": None}, go
        events = cli.events(capture / "go.jsonl")
        assert all(events.get(test) == "pass" for test in TESTS), events
        (capture / "manifest.json").write_text(json.dumps(dict(binding, go=go, original_assertions=events,
            captured_sha256={name: sha(capture / name) for name in ["calls.json", "go.jsonl"]}), indent=2) + "\n")
    manifest = json.loads((capture / "manifest.json").read_text())
    assert all(manifest[k] == v for k, v in binding.items()), "changed source/hook"
    assert manifest["go"] == {"exit": 0, "failure": None}
    assert all(manifest["original_assertions"].get(test) == "pass" for test in TESTS)
    assert manifest["captured_sha256"] == {name: sha(capture / name) for name in ["calls.json", "go.jsonl"]}, "changed capture"
    calls = json.loads((capture / "calls.json").read_text())
    validate_calls(calls)
    work = output.with_suffix("")
    work.mkdir(exist_ok=False)
    (work / "tmp").mkdir()
    request, response = work / "request.json", work / "response.json"
    request.write_text(json.dumps([c["input"] for c in calls]) + "\n")
    env.update(TMPDIR=str(work / "tmp"), D2RS_PATH_REQUEST=str(request), D2RS_PATH_RESPONSE=str(response))
    digest = sha(runner)
    cli.TRACE = ROOT
    native = cli.bounded_run([str(runner), "--ignored", "--exact", "board_output::path_api_tests::replay",
                              "--test-threads=1"], env, work, work / "native.log", 120)
    assert sha(runner) == digest, "changed runner"
    results = json.loads(response.read_text()) if native == {"exit": 0, "failure": None} and response.is_file() else []
    rows = compare(calls, results) if len(results) == len(calls) else [
        dict(test=test, outcome="native-failed", calls=count, matched_calls=0) for test, count in TESTS.items()]
    report = dict(binding, runner=str(runner), runner_sha256=digest, capture=str(capture), native=native,
                  original_assertions=manifest["original_assertions"], calls=calls, results=results, rows=rows,
                  controls=dict(total_bytes=1 << 30, file_bytes=128 << 20, minimum_free_bytes=20 << 30, deadline_seconds=600),
                  projection="Original names as byte arrays call production board_output_component, shared with UTF-8 CLI names. Original diagram name/kind/folder tree and output path; native board_output_path and validate_board_output_paths compare complete errors. Original SVG append calls adapt the output root with the existing Path::with_extension behavior and call the production contained_board_output_component / ensure_path_within helpers. Historical unsupported responses remain non-passing. Original Go assertions unchanged; no normalization. Other extensions and arbitrary raw paths are not qualified.")
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(rows, indent=2))
    return int(any(row["outcome"] not in {"match", "unsupported"} for row in rows))


if __name__ == "__main__":
    raise SystemExit(main())
