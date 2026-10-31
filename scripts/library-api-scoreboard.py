#!/usr/bin/env python3
"""Original d2lib assertions plus native configuration and compile admission.

Complete options, appearance configs, SVG bytes and errors compare.
Captures retain original assertions and have streaming 1 GiB total / 128 MiB
file / 20 GiB free guards, with finite deadlines and fresh writable outputs.
"""

import argparse
import collections
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
HOOK = "tools/library-api-scoreboard/hook_test.go"
TEST_FILES = [
    "d2lib/config_test.go",
    "d2lib/latex_limits_test.go",
    "d2lib/layout_reuse_test.go",
]
TESTS = {
    "TestApplyConfigsThemeOverridePrecedence": 2,
    "TestIsometricConfigUsesSketchPrecedence": 8,
    "TestIsometricCompilePreservesUnspecifiedConfig": 3,
    "TestDisabledIsometricRendersIdenticallyToDefault": 4,
    "TestCompileRejectsExcessiveLatexGroupNesting": 2,
    "TestCompileReusesStyleOnlyScenarioAndStepGeometry": 2,
    "TestCompileLayoutReuseRejectsGeometryChangesAndLayers": 2,
    "TestLayoutInputSignatureIgnoresOnlyGeometryNeutralStyles": 3,
    "TestCompileLayoutReuseRequiresExplicitStableResolver": 1,
    "TestCompileLayoutReusePreservesPerBoardCancellation": 2,
}


def module(name):
    spec = importlib.util.spec_from_file_location(
        name, ROOT / "scripts" / (name + ".py")
    )
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module("asset-policy-scoreboard")


def source():
    pinned = ROOT / references.MANIFEST["d2"]["checkout"]
    paths = TEST_FILES + ["d2lib/d2.go", "d2lib/layout_reuse.go"]
    return {
        "commit": references.SOURCE_COMMIT,
        "files": {p: base.sha(pinned / p) for p in paths},
        "hook": base.sha(ROOT / HOOK),
    }


def instrument(text):
    text = text.replace("applyConfigs(", "d2rsLibraryConfigs(t, ")
    text = text.replace("applyDefaults(", "d2rsLibraryDefaults(t, ")
    text = text.replace("Compile(", "d2rsLibraryCompile(t, ")
    text = text.replace(
        "compileInput(ctx, input, opts, &d2svg.RenderOpts{})",
        "d2rsLibraryCompile(t, ctx, input, opts, &d2svg.RenderOpts{})",
    )
    return text.replace(
        "if _, _, err := d2rsLibraryCompile(t, ctx, input, opts, renderOpts);",
        "if _, _, err := compileInput(ctx, input, opts, renderOpts);",
    )


def verify(capture):
    assert json.loads((capture / "source.json").read_text()) == source()
    assert json.loads((capture / "inventory.json").read_text()) == TESTS
    assert json.loads((capture / "hashes.json").read_text()) == base.hashes(capture)
    assert json.loads((capture / "run.json").read_text()) == {
        "exit": 0,
        "failure": None,
    }
    events = module("cli-scoreboard").events(capture / "go.jsonl")
    assert all(events.get(test) == "pass" for test in TESTS), events
    assert all(action != "fail" for action in events.values())
    cases = [json.loads(p.read_text()) for p in sorted((capture / "go").glob("*.json"))]
    assert len(cases) == sum(TESTS.values())
    assert len({c["name"] for c in cases}) == len(cases)
    assert collections.Counter(c["test"].split("/")[0] for c in cases) == TESTS
    assert all(events.get(c["test"]) == "pass" for c in cases)
    sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def capture_original(capture):
    pinned = references.verify_source()
    originals = {name: (pinned / name).read_bytes() for name in TEST_FILES}
    helper = references.TRACE_CHECKOUT / "d2lib/d2rs_library_test.go"
    assert not helper.exists()
    assert all(
        (references.TRACE_CHECKOUT / name).read_bytes() == data
        for name, data in originals.items()
    )
    (capture / "go").mkdir(parents=True, exist_ok=False)
    (capture / "tmp").mkdir()
    (capture / "source.json").write_text(json.dumps(source(), indent=2) + "\n")
    (capture / "inventory.json").write_text(json.dumps(TESTS, indent=2) + "\n")
    try:
        for name, data in originals.items():
            (references.TRACE_CHECKOUT / name).write_text(instrument(data.decode()))
        helper.write_bytes((ROOT / HOOK).read_bytes())
        env = dict(
            os.environ,
            TMPDIR=str(capture / "tmp"),
            GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
            GOCACHE=str(ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod",
            GOPROXY="off",
            D2RS_LIBRARY_DUMP=str(capture / "go"),
        )
        env.pop("TA", None)
        env.pop("TESTDATA_ACCEPT", None)
        result = module("cli-scoreboard").bounded_run(
            [
                "go",
                "test",
                "-ldflags=-X github.com/d2lang/d2/lib/version.Version=v0.9.0",
                "./d2lib",
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
        for name, data in originals.items():
            (references.TRACE_CHECKOUT / name).write_bytes(data)
        helper.unlink(missing_ok=True)
    (capture / "run.json").write_text(json.dumps(result) + "\n")
    assert result == {"exit": 0, "failure": None}, result
    (capture / "hashes.json").write_text(
        json.dumps(base.hashes(capture), indent=2) + "\n"
    )
    verify(capture)


def compare(cases, actual):
    assert isinstance(actual, list) and len(actual) == len(cases)
    rows = []
    for case, got in zip(cases, actual, strict=True):
        row = {"name": case["name"], "test": case["test"]}
        row["outcome"] = (
            "unsupported"
            if isinstance(got, dict) and "unsupported" in got
            else "match"
            if module("language-scoreboard").same_json(case["got"], got)
            else "mismatch"
        )
        if row["outcome"] != "match":
            row["expected"], row["actual"] = case["got"], got
        rows.append(row)
    return rows


def functions(rows):
    grouped = collections.defaultdict(list)
    for row in rows:
        grouped[row["test"].split("/")[0]].append(row["outcome"])
    return [
        {"name": name, "outcome": next((v for v in values if v != "match"), "match")}
        for name, values in sorted(grouped.items())
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--reuse", action="store_true")
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        capture_original(capture)
    if not args.output:
        return
    cases = sorted(verify(capture), key=lambda c: c["name"])
    work = args.output.parent / (args.output.stem + "-native")
    work.mkdir(exist_ok=False)
    request, log = work / "request.json", work / "actual.json"
    request.write_text(json.dumps([c["request"] for c in cases]) + "\n")
    assert args.rust and args.rust.is_file()
    digest = base.sha(args.rust)
    run = module("cli-scoreboard").bounded_run(
        [str(args.rust.resolve()), str(request.resolve())],
        os.environ.copy(),
        work,
        log,
        120,
    )
    assert base.sha(args.rust) == digest
    rows = (
        compare(cases, json.loads(log.read_text()))
        if run == {"exit": 0, "failure": None}
        else [
            {
                "name": c["name"],
                "test": c["test"],
                "outcome": run["failure"] or "rust-failed",
            }
            for c in cases
        ]
    )
    function_rows = functions(rows)
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "package": "d2lib",
        "runner_sha256": digest,
        "capture": str(capture.relative_to(ROOT)),
        "capture_hashes": base.hashes(capture),
        "captured_functions": len(TESTS),
        "operations": sum(TESTS.values()),
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "function_summary": dict(
            collections.Counter(r["outcome"] for r in function_rows)
        ),
        "function_rows": function_rows,
        "rows": rows,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
