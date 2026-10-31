#!/usr/bin/env python3
"""Replay every original master Markdown substitution call.

Complete result/work count and typed/exact errors, with all original Go assertions active.
Capture limits: 1 GiB total / 128 MiB per file / 20 GiB minimum free space.
"""
import argparse
import hashlib
import importlib.util
import json
import os
import re
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("master", ROOT / "scripts/language-master-scoreboard.py")
master = importlib.util.module_from_spec(spec)
spec.loader.exec_module(master)
RELATIVE = "lib/textmeasure/substitutions_test.go"
TESTS = {
    "TestReplaceSubstitutionsMarkdownBounded": 5,
    "TestMarkdownMatcherBoundsSuffixCandidateWork": 2,
    "TestReplaceSubstitutionsMarkdownChargesIdentityReplacementWork": 2,
    "TestReplaceSubstitutionsMarkdownLeavesUnknownPlaceholdersUnchanged": 1,
    "TestReplaceSubstitutionsMarkdownIgnoresUnreferencedVariablesInLinearTime": 1,
    "TestReplaceSubstitutionsMarkdownBoundedCancellation": 1,
    "TestReplaceSubstitutionsMarkdownMatchesManyPatternsInLinearTime": 1,
}


def hashes(capture):
    paths = [capture / name for name in ("inventory.json", "go.jsonl", "run.json", "source.json")]
    paths += sorted((capture / "go").glob("*.json"))
    assert len(paths) > 4, "empty Markdown capture"
    return {str(path.relative_to(capture)): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}


def verify(capture):
    source = json.loads((capture / "source.json").read_text())
    assert source["source_commit"] == references.SOURCE_COMMIT, "source pin changed"
    pinned = ROOT / references.MANIFEST["d2"]["checkout"] / RELATIVE
    assert source["test_sha256"] == hashlib.sha256(pinned.read_bytes()).hexdigest(), "source test changed"
    assert json.loads((capture / "run.json").read_text()) == {"exit": 0, "failure": None}, "original tests failed"
    assert json.loads((capture / "hashes.json").read_text()) == hashes(capture), "capture missing, extra or changed"
    calls = [json.loads(path.read_text()) for path in (capture / "go").glob("*.json")]
    names = [case["name"] for case in calls]
    expected = {f"{test}#{i}" for test, count in TESTS.items() for i in range(count)}
    assert len(names) == len(set(names)) and set(names) == expected, "missing, extra or duplicate original Markdown call"
    events = master.module("markdown_events", "cli-scoreboard.py").events(capture / "go.jsonl")
    assert all(events.get(test) == "pass" for test in TESTS), "original Markdown assertion failed"
    assert json.loads((capture / "inventory.json").read_text()) == [{"path": RELATIVE, "test": test} for test in TESTS], "original inventory changed"
    for case in calls:
        assert case["test"] == case["name"].split("#")[0], "call test changed"
        assert set(case["got"]) == {"result", "used", "error", "error_kind"}, "incomplete Markdown result"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--runner", type=Path, default=ROOT / "target/diagnostic/examples/master-markdown-scoreboard")
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--capture-only", action="store_true")
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        original = (references.verify_source() / RELATIVE).read_bytes()
        target = references.TRACE_CHECKOUT / RELATIVE
        helper = references.TRACE_CHECKOUT / "lib/textmeasure/d2rs_markdown_test.go"
        assert target.read_bytes() == original and not helper.exists(), "unexpected Markdown instrumentation"
        (capture / "go").mkdir(parents=True, exist_ok=False)
        (capture / "inventory.json").write_text(json.dumps([{"path": RELATIVE, "test": test} for test in TESTS], indent=2) + "\n")
        env = dict(os.environ, D2RS_MASTER_LANGUAGE_DUMP=str(capture / "go"),
                   GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"), GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off")
        env.pop("TESTDATA_ACCEPT", None)
        env.pop("TA", None)
        try:
            target.write_text(re.sub(r"\bReplaceSubstitutionsMarkdownBounded\(", "d2rsMarkdownReplace(t, ", original.decode()))
            helper.write_bytes((ROOT / "tools/language-master-scoreboard/markdown_hook_test.go").read_bytes())
            cli = master.module("markdown_capture", "cli-scoreboard.py")
            run = cli.bounded_run(["go", "test", "./lib/textmeasure", "-run", "^(" + "|".join(TESTS) + ")$", "-json", "-count=1"], env, capture, capture / "go.jsonl", 600)
        finally:
            target.write_bytes(original)
            helper.unlink(missing_ok=True)
        (capture / "run.json").write_text(json.dumps(run, indent=2) + "\n")
        assert run == {"exit": 0, "failure": None}, run
        (capture / "source.json").write_text(json.dumps({"source_commit": references.SOURCE_COMMIT, "test_sha256": hashlib.sha256(original).hexdigest()}, indent=2) + "\n")
        (capture / "hashes.json").write_text(json.dumps(hashes(capture), indent=2) + "\n")
    verify(capture)
    if not args.capture_only:
        master.score(capture, args.runner, args.output, package="lib")
        report = json.loads(args.output.read_text())
        report["uncaptured_assertions"] = [{"test": "TestMarkdownMatcherNoMatchStorageIsSparse", "contract": "internal matcher allocation count and terminal transition storage"}]
        args.output.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
