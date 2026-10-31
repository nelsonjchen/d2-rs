#!/usr/bin/env python3
"""Original imgbundler resolver/worker calls; no network policy qualification.

Uses the original mock HTTP replies and local bytes at the explicit I/O boundary.
Scheduling compares the original bounded-count assertion, never a lucky Go count.
Complete output and errors compare without normalization. Streaming budgets:
1 GiB/capture, 128 MiB/file, 20 GiB minimum free, 600 s capture deadline.
"""

import argparse
import collections
import hashlib
import json
import os
import re
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
TESTS = {
    "TestResolveReplacementEnforcesCumulativeBundleBudget": 1,
    "TestBundleWithResolverCombinesLocalAndRemoteReferences": 1,
    "TestBundleWithResolverRejectsUnsupportedFormats": 1,
    "TestResolutionFailureStopsSchedulingNewFetches": 1,
    "TestResolutionFailurePreservesCompletedReplacements": 1,
    "TestInlineRemoteContentTypeIsSafeAndCanonical": 4,
    "TestDeprecatedWrapperCacheIsDocumentScoped": 4,
}
FILES = ["lib/imgbundler/resource_limits_test.go", "lib/imgbundler/imgbundler_test.go"]
SUBTESTS = [
    "malicious_quoted_header",
    "parameterized_SVG",
    "parameterized_PNG_alias",
    "XML-sensitive_subtype",
]


def module(name):
    spec = spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    value = module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source():
    pinned = ROOT / references.MANIFEST["d2"]["checkout"]
    paths = FILES + ["lib/imgbundler/imgbundler.go"]
    return {
        "commit": references.SOURCE_COMMIT,
        "files": {p: sha(pinned / p) for p in paths},
        "hook": sha(ROOT / "tools/imgbundler-resolver-scoreboard/hook_test.go"),
    }


def hashes(capture):
    paths = [
        capture / n for n in ["source.json", "inventory.json", "go.jsonl", "run.json"]
    ]
    paths += sorted((capture / "go").glob("*.json"))
    return {str(p.relative_to(capture)): sha(p) for p in paths}


def expected():
    result = {
        f"{t}#{i}"
        for t, count in TESTS.items()
        if t != "TestInlineRemoteContentTypeIsSafeAndCanonical"
        for i in range(count)
    }
    result.update(
        f"TestInlineRemoteContentTypeIsSafeAndCanonical/{s}#0" for s in SUBTESTS
    )
    return result


def verify(capture):
    assert json.loads((capture / "source.json").read_text()) == source()
    assert json.loads((capture / "hashes.json").read_text()) == hashes(capture)
    assert json.loads((capture / "run.json").read_text()) == {
        "exit": 0,
        "failure": None,
    }
    events = module("cli-scoreboard").events(capture / "go.jsonl")
    cases = [json.loads(p.read_text()) for p in sorted((capture / "go").glob("*.json"))]
    assert len(cases) == len(expected()) and {c["name"] for c in cases} == expected()
    assert all(events.get(t) == "pass" for t in TESTS), events
    assert all(
        events.get("TestInlineRemoteContentTypeIsSafeAndCanonical/" + s) == "pass"
        for s in SUBTESTS
    )
    assert json.loads((capture / "inventory.json").read_text()) == TESTS
    sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def capture_original(capture):
    pinned = references.verify_source()
    originals = {p: (pinned / p).read_bytes() for p in FILES}
    helper = (
        references.TRACE_CHECKOUT / "lib/imgbundler/d2rs_imgbundler_resolver_test.go"
    )
    assert not helper.exists()
    for p, original in originals.items():
        assert (references.TRACE_CHECKOUT / p).read_bytes() == original
    (capture / "go").mkdir(parents=True, exist_ok=False)
    (capture / "source.json").write_text(json.dumps(source(), indent=2) + "\n")
    (capture / "inventory.json").write_text(json.dumps(TESTS, indent=2) + "\n")
    try:
        for p, original in originals.items():
            parts = re.split(r"(?=^func Test)", original.decode(), flags=re.MULTILINE)
            for i, part in enumerate(parts):
                if any(part.startswith("func " + t + "(") for t in TESTS):
                    for old, new in [
                        ("imageasset.New(", "d2rsNew(t, "),
                        ("resolveReplacement(", "d2rsReplacement(t, "),
                        ("BundleWithResolver(", "d2rsBundle(t, "),
                        ("runResolverWorkersWithLimit(", "d2rsWorkersLimit(t, "),
                        ("runResolverWorkers(", "d2rsWorkers(t, "),
                        ("bundleRemoteForTest(", "d2rsLegacy(t, "),
                    ]:
                        part = part.replace(old, new)
                    parts[i] = part
            (references.TRACE_CHECKOUT / p).write_text("".join(parts))
        helper.write_bytes(
            (ROOT / "tools/imgbundler-resolver-scoreboard/hook_test.go").read_bytes()
        )
        env = dict(
            os.environ,
            D2RS_IMGBUNDLER_RESOLVER_DUMP=str(capture / "go"),
            GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
            GOCACHE=str(ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod",
            GOPROXY="off",
        )
        env.pop("TA", None)
        env.pop("TESTDATA_ACCEPT", None)
        result = module("cli-scoreboard").bounded_run(
            [
                "go",
                "test",
                "./lib/imgbundler",
                "-run",
                "^(" + "|".join(sorted(TESTS)) + ")$",
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
        for p, original in originals.items():
            (references.TRACE_CHECKOUT / p).write_bytes(original)
        helper.unlink(missing_ok=True)
    (capture / "run.json").write_text(json.dumps(result) + "\n")
    assert result == {"exit": 0, "failure": None}, result
    (capture / "hashes.json").write_text(json.dumps(hashes(capture), indent=2) + "\n")
    verify(capture)


def classify(case, actual):
    if "unsupported" in actual:
        return "unsupported"
    if case["test"] == "TestResolutionFailureStopsSchedulingNewFetches":
        # Upstream tests a resolution failure, original SVG and the worker ceiling.
        # The raw first-failure URL is scheduler-dependent and remains in both logs.
        error = actual.get("error", "")
        ok = (
            actual.get("output") == case["got"]["output"]
            and actual.get("requests_within_worker_limit") is True
            and "malformed" in error
            and "context canceled" not in error
        )
        return "assertion-match" if ok else "mismatch"
    return "match" if actual == case["got"] else "mismatch"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument(
        "--batch",
        action="store_true",
        help="Execute all calls in one native process, including consecutive document-cache calls",
    )
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        capture_original(capture)
    if args.output:
        assert args.rust
        cases = verify(capture)
        work = args.output.parent / (args.output.stem + "-native")
        work.mkdir(exist_ok=False)
        cli = module("cli-scoreboard")
        runner_hash = sha(args.rust)
        rows = []
        batch_actual = None
        if args.batch:
            # Preserve the original invocation order within a test, especially the
            # repeated cache calls. Other tests have independent original fixtures.
            cases.sort(key=lambda c: c["name"])
            request, log = work / "batch.request.json", work / "batch.log"
            request.write_text(json.dumps([case["request"] for case in cases]))
            run = cli.bounded_run(
                [str(args.rust.resolve()), str(request.resolve())],
                os.environ.copy(),
                work,
                log,
                60,
            )
            assert run == {"exit": 0, "failure": None}, run
            batch_actual = json.loads(log.read_text())
            assert isinstance(batch_actual, list) and len(batch_actual) == len(cases)
        for index, case in enumerate(cases):
            key = hashlib.sha256(case["name"].encode()).hexdigest()
            request, log = work / (key + ".request.json"), work / (key + ".log")
            request.write_text(json.dumps(case["request"]))
            run = (
                {"exit": 0, "failure": None}
                if args.batch
                else cli.bounded_run(
                    [str(args.rust.resolve()), str(request.resolve())],
                    os.environ.copy(),
                    work,
                    log,
                    30,
                )
            )
            row = {"name": case["name"], "test": case["test"]}
            if run["failure"] or run["exit"]:
                row["outcome"] = run["failure"] or "rust-failed"
            else:
                try:
                    actual = (
                        batch_actual[index]
                        if args.batch
                        else json.loads(log.read_text())
                    )
                    if args.batch:
                        log.write_text(json.dumps(actual) + "\n")
                except ValueError:
                    actual = {"invalid_output": True}
                row["outcome"] = classify(case, actual)
                if row["outcome"] == "mismatch":
                    row["differences"] = module("language-scoreboard").differences(
                        case["got"], actual
                    )
            rows.append(row)
        assert sha(args.rust) == runner_hash
        report = {
            "source_commit": references.SOURCE_COMMIT,
            "runner_sha256": runner_hash,
            "capture_hashes": hashes(capture),
            "summary": dict(collections.Counter(r["outcome"] for r in rows)),
            "captured_functions": len(TESTS),
            "same_native_process": args.batch,
            "rows": rows,
        }
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(report["summary"])


if __name__ == "__main__":
    main()
