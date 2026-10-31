#!/usr/bin/env python3
"""Inventory master-added language tests and score compiler calls from their original bodies.

Only the trace checkout is instrumented. Go assertions must pass. Each call
compares full diagnostics, compiler graph and config without normalization.
Internal contracts without a native adapter remain explicitly uncovered.
Capture budget: 1 GiB total / 128 MiB per file / 20 GiB minimum volume free.
"""
from __future__ import annotations
import argparse
import collections
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import references

ROOT = Path(__file__).resolve().parents[1]
SOURCE = (ROOT / references.MANIFEST["d2"]["checkout"]).resolve()
PACKAGES = ("d2parser", "d2ast", "d2ir", "d2compiler", "d2graph", "d2format", "d2oracle", "d2lsp")

def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value

def inventory():
    diff = subprocess.check_output(["git", "diff", "v0.9.0.." + references.SOURCE_COMMIT, "--", "*_test.go"], cwd=SOURCE, text=True)
    rows, path = [], None
    for line in diff.splitlines():
        if line.startswith("+++ b/"): path = line[6:]
        if path and (path.split("/")[0] in PACKAGES or path.startswith("lib/")):
            match = re.match(r"\+func (Test\w+)\(t \*testing.T\)", line)
            if match: rows.append({"path":path, "test":match[1]})
    return rows

def install(tests):
    references.verify_source()
    trace = references.TRACE_CHECKOUT
    assert subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=trace, text=True).strip() == references.SOURCE_COMMIT
    hook = ROOT / "tools/language-master-scoreboard/compiler_hook_test.go"
    (trace / "d2compiler/d2rs_master_language_test.go").write_bytes(hook.read_bytes())
    for path in {row["path"] for row in tests if row["path"].startswith("d2compiler/")}:
        text = (trace / path).read_text()
        if text.startswith("package d2compiler_test"):
            # The adapter consumes the exact reader passed by the original test.
            text = text.replace("d2compiler.Compile(", "d2rsMasterCompile(t, ")
            if "d2compiler." not in text:
                text += '\nvar _ = d2compiler.Compile // d2rs master capture import anchor\n'
            (trace / path).write_text(text)

def uninstall(tests):
    """Detach only this lane's wrapper; preserve all other trace hooks."""
    trace = references.TRACE_CHECKOUT
    for path in {row["path"] for row in tests if row["path"].startswith("d2compiler/")}:
        text = (trace / path).read_text()
        if "d2rsMasterCompile(t, " not in text:
            continue
        text = text.replace("d2rsMasterCompile(t, ", "d2compiler.Compile(")
        text = text.replace('\nvar _ = d2compiler.Compile // d2rs master capture import anchor\n', '')
        (trace / path).write_text(text)
    (trace / "d2compiler/d2rs_master_language_test.go").unlink(missing_ok=True)

def score(capture, runner, output, package="d2compiler"):
    runner_hash = hashlib.sha256(runner.read_bytes()).hexdigest()
    cli = module("master_cli", "cli-scoreboard.py")
    lang = module("master_lang", "language-scoreboard.py")
    events = cli.events(capture / "go.jsonl")
    rows = []
    output.parent.mkdir(parents=True, exist_ok=True)
    native_dir = output.parent / (output.stem + "-native")
    native_dir.mkdir(exist_ok=False)
    cases = sorted((capture / "go").glob("*.json"))
    assert cases, "empty capture is not a passing scoreboard"
    for path in cases:
        case = json.loads(path.read_text())
        request = native_dir / (path.stem + ".request.json")
        request.write_text(json.dumps(case["request"]))
        log = native_dir / (path.stem + ".log")
        result = cli.bounded_run([str(runner.resolve()), str(request.resolve())], os.environ.copy(), native_dir, log, 10)
        row = {"name":case["name"], "test":case["test"], "go_test":events.get(case["test"], "missing-event")}
        if row["go_test"] != "pass": row["outcome"] = "go-" + row["go_test"]
        elif result["failure"] or result["exit"]: row["outcome"] = result["failure"] or "rust-failed"
        else:
            try: actual = json.loads(log.read_text())
            except (ValueError, OSError): actual = {"invalid_output":True}
            row["outcome"] = "unsupported" if "unsupported" in actual else "match" if lang.same_json(case["got"], actual) else "mismatch"
            if row["outcome"] != "match": row["differences"] = lang.differences(case["got"], actual)
        rows.append(row)
    tests = json.loads((capture / "inventory.json").read_text())
    captured = {name for row in rows for name in (row["test"], row["test"].split("/")[0])}
    uncovered = [dict(row, outcome="uncovered") for row in tests if row["test"] not in captured or not row["path"].startswith(package + "/")]
    uncovered_subtests = []
    captured_tests = {row["test"] for row in rows}
    selected = {row["test"] for row in tests if row["path"].startswith(package + "/")}
    for test, event in sorted(events.items()):
        if test.split("/")[0] in selected and test not in captured_tests and not any(name.startswith(test + "/") for name in events):
            if not any(row["test"] == test for row in uncovered):
                uncovered_subtests.append({"path":package, "test":test, "go_test":event, "outcome":"uncovered"})
    assert hashlib.sha256(runner.read_bytes()).hexdigest() == runner_hash, "native runner changed during scoring; rerun with a stable binary"
    report = {"source_commit":references.SOURCE_COMMIT, "runner_sha256":runner_hash,
              "capture_sha256":hashlib.sha256(b"".join(path.read_bytes() for path in cases)).hexdigest(),
              "budget_bytes":1024**3, "per_file_bytes":128*1024**2,
              "summary":dict(collections.Counter(row["outcome"] for row in rows)), "rows":rows,
              "uncovered_tests":uncovered, "uncovered_subtests":uncovered_subtests}
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"], "uncovered tests", len(uncovered))

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--runner", type=Path, default=ROOT / "target/oracle/examples/master-language-scoreboard")
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--capture-only", action="store_true")
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        tests = inventory(); install(tests)
        (capture / "go").mkdir(parents=True, exist_ok=False)
        (capture / "inventory.json").write_text(json.dumps(tests, indent=2) + "\n")
        names = [row["test"] for row in tests if row["path"].startswith("d2compiler/")]
        env = dict(os.environ, D2RS_MASTER_LANGUAGE_DUMP=str(capture / "go"),
                   GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"), GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off")
        env.pop("TESTDATA_ACCEPT", None); env.pop("TA", None)
        cli = module("master_capture", "cli-scoreboard.py")
        try:
            run = cli.bounded_run(["go", "test", "./d2compiler", "-run", "^(" + "|".join(names) + ")$", "-json", "-count=1", "-parallel=4"], env, capture, capture / "go.jsonl", 600)
        finally:
            uninstall(tests)
        (capture / "run.json").write_text(json.dumps(run, indent=2) + "\n")
        if run["failure"] or run["exit"]: raise RuntimeError(run)
    if not args.capture_only:
        score(capture, args.runner, args.output)

if __name__ == "__main__": main()
