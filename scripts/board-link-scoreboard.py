#!/usr/bin/env python3
"""Original CLI node/connection board-link calls; complete target/error bytes."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
TESTS = {"TestRelinkIncludesConnections": 2}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate_calls(calls):
    assert len(calls) == 2, "incomplete original calls"
    assert [c["test"] for c in calls] == ["TestRelinkIncludesConnections"] * 2
    assert [c["input"]["op"] for c in calls] == ["relink", "link"]
    assert set(calls[0]["input"]["diagram"]) == set(calls[0]["result"]["diagram"])


def compare(calls, results):
    validate_calls(calls)
    assert len(results) == len(calls), "missing/extra native responses"
    matched = sum(type_equal(c["result"], r) for c, r in zip(calls, results))
    return [dict(test="TestRelinkIncludesConnections", calls=2, matched_calls=matched,
                 outcome="match" if matched == 2 else "mismatch")]


def type_equal(a, b):
    # JSON bools must not compare as numbers; complete fields and arrays remain.
    if type(a) is not type(b):
        return False
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(type_equal(a[k], b[k]) for k in a)
    if isinstance(a, list):
        return len(a) == len(b) and all(type_equal(x, y) for x, y in zip(a, b))
    return a == b


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
    hook = ROOT / "tools/board-path-scoreboard/links_hook_test.go"
    binding = dict(source_commit=references.SOURCE_COMMIT, package="d2cli", hook_sha256=sha(hook),
                   test_sha256=hashlib.sha256(original).hexdigest(),
                   implementation_sha256=sha(source / "d2cli/main.go"))
    env = dict(os.environ, GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
               GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off")
    if not args.reuse:
        capture.mkdir(exist_ok=False)
        (capture / "tmp").mkdir()
        target = references.TRACE_CHECKOUT / "d2cli/board_output_test.go"
        helper = target.parent / "d2rs_board_links_test.go"
        assert target.read_bytes() == original and not helper.exists(), "concurrent source mutation"
        text = original.decode()
        assert text.count('relink("root", diagram, links)') == 1
        text = text.replace('relink("root", diagram, links)', 'd2rsBoardRelink(t, "root", diagram, links)')
        assert text.count('got := boardOutputLink(') == 1
        text = text.replace('got := boardOutputLink(', 'got := d2rsBoardOutputLink(t, ')
        env.update(TMPDIR=str(capture / "tmp"), D2RS_LINK_CAPTURE=str(capture))
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
    env.update(TMPDIR=str(work / "tmp"), D2RS_LINK_REQUEST=str(request), D2RS_LINK_RESPONSE=str(response))
    digest = sha(runner)
    cli.TRACE = ROOT
    native = cli.bounded_run([str(runner), "--ignored", "--exact", "board_output::link_api_tests::replay",
                              "--test-threads=1"], env, work, work / "native.log", 120)
    assert sha(runner) == digest, "changed runner"
    results = json.loads(response.read_text()) if native == {"exit": 0, "failure": None} and response.is_file() else []
    rows = compare(calls, results) if len(results) == len(calls) else [
        dict(test=test, outcome="native-failed", calls=count, matched_calls=0) for test, count in TESTS.items()]
    report = dict(binding, runner=str(runner), runner_sha256=digest, capture=str(capture), native=native,
                  original_assertions=manifest["original_assertions"], calls=calls, results=results, rows=rows,
                  controls=dict(total_bytes=1 << 30, file_bytes=128 << 20, minimum_free_bytes=20 << 30, deadline_seconds=600),
                  projection="Original target links adapt into production node styles and graph edge links; shared relink_document_links loops and relative_board_output_link / board_output_link perform all rewriting. Other target fields remain unchanged. Original assertions active. No normalization; finite Unix path contract, not recursive board/error coverage.")
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(rows, indent=2))
    return int(any(row["outcome"] != "match" for row in rows))


if __name__ == "__main__":
    raise SystemExit(main())
