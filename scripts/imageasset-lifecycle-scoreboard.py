#!/usr/bin/env python3
"""Observe original resource encoding and resolver transport lifetime assertions.

Full returned bytes and ownership/close observations compare. Source assertions
remain active. Captures have 1 GiB/128 MiB/20 GiB streaming guards and deadlines.
"""

import argparse
import collections
import json
import os
import re
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
TESTS = {
    "TestResourceDataURIContext": 5,
    "TestResolverClosesOnlyOwnedHTTPTransport": 10,
}


def module(name):
    spec = spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    value = module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def sha(path):
    return module("asset-policy-scoreboard").sha(path)


def source():
    pinned = references.verify_source()
    files = ["lib/imageasset/imageasset.go", "lib/imageasset/imageasset_test.go"]
    return {
        "commit": references.SOURCE_COMMIT,
        "files": {p: sha(pinned / p) for p in files},
        "hook": sha(ROOT / "tools/imageasset-lifecycle-scoreboard/hook_test.go"),
    }


def hashes(capture):
    files = [
        capture / p for p in ["source.json", "inventory.json", "go.jsonl", "run.json"]
    ]
    files += sorted((capture / "go").glob("*.json"))
    return {str(p.relative_to(capture)): sha(p) for p in files}


def verify(capture):
    assert json.loads((capture / "source.json").read_text()) == source()
    assert json.loads((capture / "inventory.json").read_text()) == TESTS
    assert json.loads((capture / "hashes.json").read_text()) == hashes(capture)
    assert json.loads((capture / "run.json").read_text()) == {
        "exit": 0,
        "failure": None,
    }
    events = module("cli-scoreboard").events(capture / "go.jsonl")
    assert all(events.get(t) == "pass" for t in TESTS) and "fail" not in events.values()
    cases = [json.loads(p.read_text()) for p in sorted((capture / "go").glob("*.json"))]
    validate_cases(cases)
    sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def validate_cases(cases):
    assert {c["name"] for c in cases} == set(TESTS) and len(cases) == len(TESTS)
    assert all(len(c["operations"]) == TESTS[c["name"]] for c in cases)
    for case in cases:
        ops = collections.Counter(op["request"]["op"] for op in case["operations"])
        assert ops == (
            {"data-uri": 5}
            if case["name"] == "TestResourceDataURIContext"
            else {"new": 4, "close": 6}
        )


def outcome(expected, got):
    if got is None:
        return "rust-failed"
    if isinstance(got, dict) and "unsupported" in got:
        return "unsupported"
    return (
        "match"
        if module("imageasset-scoreboard").compare(expected, got)
        else "mismatch"
    )


def capture_original(capture):
    pinned = references.verify_source()
    path = references.TRACE_CHECKOUT / "lib/imageasset/imageasset_test.go"
    original = path.read_bytes()
    assert original == (pinned / "lib/imageasset/imageasset_test.go").read_bytes()
    hook = path.parent / "d2rs_lifecycle_test.go"
    assert not hook.exists()
    text = original.decode()
    for name in TESTS:
        match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
        assert match
        body = match[0].replace("{\n", "{\n\td2rsLifecycleStart(t)\n", 1)
        body = re.sub(r"\b(\w+)\.DataURIContext\(", r"d2rsLifecycleURI(t, \1, ", body)
        body = body.replace("New(Options{", "d2rsLifecycleNew(t, Options{")
        body = re.sub(
            r"\b(\w+)\.CloseIdleConnections\(\)", r"d2rsLifecycleClose(t, \1)", body
        )
        text = text[: match.start()] + body + text[match.end() :]
    (capture / "go").mkdir(parents=True)
    (capture / "tmp").mkdir()
    (capture / "source.json").write_text(json.dumps(source(), indent=2) + "\n")
    (capture / "inventory.json").write_text(json.dumps(TESTS) + "\n")
    env = dict(
        os.environ,
        TMPDIR=str(capture / "tmp"),
        D2RS_IMAGEASSET_LIFECYCLE=str(capture / "go"),
        GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
        GOCACHE=str(ROOT / ".reference/go-build"),
        GOFLAGS="-mod=mod",
        GOPROXY="off",
    )
    try:
        path.write_text(text)
        hook.write_bytes(
            (ROOT / "tools/imageasset-lifecycle-scoreboard/hook_test.go").read_bytes()
        )
        result = module("cli-scoreboard").bounded_run(
            [
                "go",
                "test",
                "./lib/imageasset",
                "-run",
                "^(" + "|".join(TESTS) + ")$",
                "-json",
                "-count=1",
                "-parallel=1",
            ],
            env,
            capture,
            capture / "go.jsonl",
            600,
        )
    finally:
        path.write_bytes(original)
        hook.unlink(missing_ok=True)
    (capture / "run.json").write_text(json.dumps(result) + "\n")
    assert result == {"exit": 0, "failure": None}, result
    (capture / "hashes.json").write_text(json.dumps(hashes(capture), indent=2) + "\n")
    verify(capture)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        assert not capture.exists()
        capture_original(capture)
    cases = verify(capture)
    if not args.output:
        print("verified original tests", len(cases))
        return
    assert args.rust and args.rust.is_file() and not args.output.exists()
    work = args.output.parent / (args.output.stem + "-native")
    work.mkdir(exist_ok=False)
    request, log = work / "request.json", work / "actual.json"
    request.write_text(
        json.dumps(
            [
                {
                    "name": c["name"],
                    "operations": [op["request"] for op in c["operations"]],
                }
                for c in cases
            ]
        )
    )
    digest = sha(args.rust)
    run = module("cli-scoreboard").bounded_run(
        [str(args.rust.resolve()), str(request.resolve())],
        os.environ.copy(),
        work,
        log,
        60,
    )
    actual = (
        json.loads(log.read_text()) if run == {"exit": 0, "failure": None} else None
    )
    assert actual is None or len(actual) == len(cases)
    rows = []
    for i, c in enumerate(cases):
        expected = [op["got"] for op in c["operations"]]
        got = actual[i] if actual is not None else None
        result = outcome(expected, got)
        row = {
            "name": c["name"],
            "outcome": result,
            "input_sha256": module("imageasset-scoreboard").value_sha(
                [op["request"] for op in c["operations"]]
            ),
            "expected_sha256": module("imageasset-scoreboard").value_sha(expected),
        }
        if result == "mismatch":
            row["differences"] = module("language-scoreboard").differences(
                expected, got
            )
        rows.append(row)
    assert sha(args.rust) == digest
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "runner_sha256": digest,
        "capture_hashes": hashes(capture),
        "run": run,
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
