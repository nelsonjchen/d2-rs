#!/usr/bin/env python3
"""Master-added parser tests: full partial AST, typed errors and exact diagnostics.

Original Go assertions remain active. Capture: 1 GiB total, 128 MiB per file,
20 GiB free, streaming enforcement shared with the other language tiers.
"""
import argparse
import hashlib
import importlib.util
import json
import os
import sys
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("master", ROOT / "scripts/language-master-scoreboard.py")
master = importlib.util.module_from_spec(spec)
spec.loader.exec_module(master)
RELATIVE = "d2parser/parse_test.go"


def hashes(capture):
    paths = [capture / name for name in ("inventory.json", "go.jsonl", "run.json", "source.json")]
    paths += sorted((capture / "go").glob("*.json"))
    assert len(paths) > 4, "empty parser capture"
    return {str(path.relative_to(capture)): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}


def verify(capture):
    source = json.loads((capture / "source.json").read_text())
    assert source["source_commit"] == references.SOURCE_COMMIT, "source pin changed"
    pinned = ROOT / references.MANIFEST["d2"]["checkout"] / RELATIVE
    assert source["test_sha256"] == hashlib.sha256(pinned.read_bytes()).hexdigest(), "parser source changed"
    assert json.loads((capture / "run.json").read_text()) == {"exit": 0, "failure": None}, "source tests failed"
    assert json.loads((capture / "hashes.json").read_text()) == hashes(capture), "capture changed or incomplete"
    calls = [json.loads(path.read_text()) for path in (capture / "go").glob("*.json")]
    assert len({row["name"] for row in calls}) == len(calls), "duplicate parser call"
    tests = json.loads((capture / "inventory.json").read_text())
    assert {row["test"].split("/")[0] for row in calls} == {row["test"] for row in tests}, "missing original test"
    events = master.module("parser_events", "cli-scoreboard.py").events(capture / "go.jsonl")
    selected = {row["test"] for row in tests}
    leaves = {name for name in events if name.split("/")[0] in selected
              and not any(child.startswith(name + "/") for child in events)}
    assert leaves == {row["test"] for row in calls}, "missing original leaf call"
    assert all(events[name] == "pass" for name in leaves), "original assertion failed"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--runner", type=Path, default=ROOT / "target/oracle/examples/master-parser-scoreboard")
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--capture-only", action="store_true")
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        pinned = (references.verify_source() / RELATIVE).read_bytes()
        target = references.TRACE_CHECKOUT / RELATIVE
        original = target.read_bytes()
        helper = references.TRACE_CHECKOUT / "d2parser/d2rs_master_parser_test.go"
        # Preserve the existing golden observer; it is outside our selected tests.
        golden_hook = b'\t\t\td2rsLanguageDump(t, map[string]interface{}{"input":tc.text, "path":d2Path}, got, err)\n'
        assert original.replace(golden_hook, b"") == pinned and not helper.exists(), "unexpected parser instrumentation"
        tests = [row for row in master.inventory() if row["path"] == RELATIVE]
        assert tests, "empty original test inventory"
        (capture / "go").mkdir(parents=True, exist_ok=False)
        (capture / "inventory.json").write_text(json.dumps(tests, indent=2) + "\n")
        env = dict(os.environ, D2RS_MASTER_LANGUAGE_DUMP=str(capture / "go"),
                   GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"), GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off")
        env.pop("TESTDATA_ACCEPT", None)
        env.pop("TA", None)
        cli = master.module("parser_capture", "cli-scoreboard.py")
        try:
            target.write_text(original.decode().replace("d2parser.Parse(", "d2rsMasterParse(t, ").replace("d2parser.ParseContext(", "d2rsMasterParseContext(t, "))
            helper.write_bytes((ROOT / "tools/language-master-scoreboard/parser_hook_test.go").read_bytes())
            run = cli.bounded_run(["go", "test", "./d2parser", "-run", "^(" + "|".join(row["test"] for row in tests) + ")$", "-json", "-count=1"], env, capture, capture / "go.jsonl", 600)
        finally:
            target.write_bytes(original)
            helper.unlink(missing_ok=True)
        (capture / "run.json").write_text(json.dumps(run, indent=2) + "\n")
        assert run == {"exit": 0, "failure": None}, run
        (capture / "source.json").write_text(json.dumps({"source_commit": references.SOURCE_COMMIT, "test_sha256": hashlib.sha256(pinned).hexdigest(), "trace_test_sha256": hashlib.sha256(original).hexdigest()}, indent=2) + "\n")
        (capture / "hashes.json").write_text(json.dumps(hashes(capture), indent=2) + "\n")
    verify(capture)
    if not args.capture_only:
        master.score(capture, args.runner, args.output, package="d2parser")


if __name__ == "__main__":
    # A supported D2 map level nests several JSON boxes and comparator frames.
    # Keep the same strict comparator; allow the full boundary AST to traverse.
    sys.setrecursionlimit(10000)
    main()
