#!/usr/bin/env python3
"""Observe five original CLI resolver tests, then replay input-only API events."""
import argparse
import collections
import hashlib
import importlib.util
import json
import os
import struct
from pathlib import Path

import references

ROOT = references.ROOT
TESTS = ["TestDagreLayoutOptions", "TestELKLayoutOptions", "TestTALALayoutOptions",
         "TestTALALayoutSnapshotsSeeds", "TestConcurrentResolvedLayouts"]
FILES = ["layout_dagre_test.go", "layout_elk_test.go", "layout_options_test.go"]


def cli():
    spec = importlib.util.spec_from_file_location("resolver_cli", ROOT / "scripts/cli-scoreboard.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def requests(calls):
    # Expected outputs never enter the native request.
    return [{k: v for k, v in call.items() if k not in {"result", "error"}} for call in calls]


def equal(left, right):
    """Complete typed JSON equality, including every f64 bit and signed zero."""
    if type(left) in (int, float) and type(right) in (int, float):
        return struct.pack(">d", float(left)) == struct.pack(">d", float(right))
    if type(left) != type(right):
        return False
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(equal(left[k], right[k]) for k in left)
    if isinstance(left, list):
        return len(left) == len(right) and all(equal(a, b) for a, b in zip(left, right))
    return left == right


def compare(calls, results, execution, go_events):
    assert calls and {c["test"].split("/")[0] for c in calls} == set(TESTS), "incomplete original functions"
    assert all(go_events.get(t) == "pass" for t in TESTS), "original Go assertions failed"
    assert len(calls) == len(results), "missing/extra native event"
    expected_counts = dict(create=13, resolve=36, layout=40)
    # Bind original cardinalities, including every concurrent resolve and call.
    assert dict(collections.Counter(c["op"] for c in calls)) == expected_counts, "changed original call inventory"
    for test, counts in zip(TESTS, [(3, 4, 5), (3, 4, 5), (2, 2, 3), (2, 2, 3), (3, 24, 24)]):
        assert collections.Counter(c["op"] for c in calls if c["test"].split("/")[0] == test) == dict(zip(["create", "resolve", "layout"], counts)), "changed original function inventory"
    for name in ["dagre", "elk", "tala"]:
        assert collections.Counter(c["op"] for c in calls if c["test"] == "TestConcurrentResolvedLayouts/" + name) == dict(create=1, resolve=8, layout=8), "changed concurrent call inventory"
    rows = []
    for test in TESTS:
        indices = [i for i, call in enumerate(calls) if call["test"].split("/")[0] == test]
        matched = 0
        for i in indices:
            call, result = calls[i], results[i]
            expected = call.get("result", {"error": call.get("error", "")})
            if equal(result, expected):
                matched += 1
        rows.append(dict(test=test, events=len(indices), matched_events=matched,
                         outcome="match" if matched == len(indices) and execution == {"exit": 0, "failure": None} else "mismatch"))
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--reuse", action="store_true")
    args = parser.parse_args()
    source = references.verify_source()
    capture, output = args.capture.resolve(), args.output.resolve()
    assert not output.exists()
    helper = references.TRACE_CHECKOUT / "d2cli/d2rs_resolver_test.go"
    hook = ROOT / "tools/layout-resolver-scoreboard/hook_test.go"
    runner = cli()
    if not args.reuse:
        capture.mkdir(exist_ok=False)
        (capture / "tmp").mkdir()
        originals = {name: (source / "d2cli" / name).read_bytes() for name in FILES}
        assert not helper.exists()
        for name, original in originals.items():
            assert (references.TRACE_CHECKOUT / "d2cli" / name).read_bytes() == original, "concurrent source edit"
        try:
            for name, original in originals.items():
                assert original.decode().count("LayoutResolver(t.Context(),") == {FILES[0]: 3, FILES[1]: 3, FILES[2]: 5}[name]
                (references.TRACE_CHECKOUT / "d2cli" / name).write_text(original.decode().replace("LayoutResolver(t.Context(),", "d2rsLayoutResolver(t, t.Context(),"))
            helper.write_bytes(hook.read_bytes())
            env = {k: v for k, v in os.environ.items() if not k.startswith(("D2_", "D2RS_"))}
            env.update(GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"), GOCACHE=str(ROOT / ".reference/go-build"),
                       GOFLAGS="-mod=mod", GOPROXY="off", TMPDIR=str(capture / "tmp"), GOTMPDIR=str(capture / "tmp"),
                       D2RS_RESOLVER_CAPTURE=str(capture))
            execution = runner.bounded_run(["go", "test", "./d2cli", "-run", "^(" + "|".join(TESTS) + ")$", "-json", "-count=1", "-parallel=1"], env, capture, capture / "go.jsonl", 600)
            (capture / "execution.json").write_text(json.dumps(execution) + "\n")
        finally:
            for name, original in originals.items():
                (references.TRACE_CHECKOUT / "d2cli" / name).write_bytes(original)
            helper.unlink(missing_ok=True)
        (capture / "manifest.json").write_text(json.dumps(dict(source_commit=references.SOURCE_COMMIT, hook_sha256=sha(hook),
            sources={name: hashlib.sha256(data).hexdigest() for name, data in originals.items()},
            files={str(p.relative_to(capture)): sha(p) for p in capture.rglob("*") if p.is_file()}), indent=2) + "\n")
    manifest = json.loads((capture / "manifest.json").read_text())
    assert manifest["source_commit"] == references.SOURCE_COMMIT and manifest["hook_sha256"] == sha(hook)
    assert all(sha(source / "d2cli" / p) == digest for p, digest in manifest["sources"].items())
    assert all(sha(capture / p) == digest for p, digest in manifest["files"].items())
    assert {str(p.relative_to(capture)) for p in capture.rglob("*") if p.is_file()} == set(manifest["files"]) | {"manifest.json"}, "changed capture inventory"
    execution = json.loads((capture / "execution.json").read_text())
    assert execution == {"exit": 0, "failure": None}, execution
    calls = json.loads((capture / "calls.json").read_text())
    events = runner.events(capture / "go.jsonl")
    report = dict(source_commit=references.SOURCE_COMMIT, package="d2cli", capture=str(capture),
                  calls=calls, events=events, go_execution=execution, manifest=manifest)
    if args.rust:
        replay = output.with_suffix(".replay")
        replay.mkdir(exist_ok=False)
        (replay / "tmp").mkdir()
        request = replay / "requests.json"
        request.write_text(json.dumps(requests(calls)) + "\n")
        binary = args.rust.resolve()
        digest = sha(binary)
        runner.TRACE = ROOT
        response = replay / "responses.json"
        native = runner.bounded_run([str(binary), "--ignored", "--exact", "layout_resolver::api_tests::replay", "--test-threads=1"],
            dict(os.environ, TMPDIR=str(replay / "tmp"), D2RS_RESOLVER_REQUEST=str(request), D2RS_RESOLVER_RESPONSE=str(response)), replay, replay / "native.log", 120)
        assert sha(binary) == digest
        results = json.loads(response.read_text()) if native == {"exit": 0, "failure": None} and response.is_file() else []
        report.update(rust=str(binary), rust_sha256=digest, native_execution=native, results=results,
                      rows=compare(calls, results, native, events) if len(results) == len(calls) else [dict(test=t, outcome="execution-failed") for t in TESTS])
    else:
        report["rows"] = [dict(test=t, outcome="not-replayed") for t in TESTS]
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(dict(events=len(calls), counts=dict(collections.Counter(c["op"] for c in calls)), rows=report["rows"]), indent=2))
    return int(bool(args.rust) and any(row["outcome"] != "match" for row in report["rows"]))


if __name__ == "__main__":
    raise SystemExit(main())
