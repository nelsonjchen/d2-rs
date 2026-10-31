#!/usr/bin/env python3
"""Observe original netpolicy assertions, then replay the native dial boundary.

Complete errors, lookup arguments and numeric dial histories compare exactly.
No external network or expected decisions enter native requests. Captures use
1 GiB total / 128 MiB file / 20 GiB free streaming guards and finite deadlines.
Client/transport tests remain explicitly outside this four-function batch.
"""

import argparse
import collections

# Reuse the existing bounded capture, hashing and event helpers.
import importlib.util
import json
import os
from pathlib import Path

import references

_spec = importlib.util.spec_from_file_location(
    "asset_policy", Path(__file__).with_name("asset-policy-scoreboard.py")
)
base = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(base)
ROOT = references.ROOT
TESTS = {
    "TestNonPublicAddressRanges": 25,
    "TestValidatedDialUsesOnlyResolvedNumericAddress": 1,
    "TestValidatedDialRejectsMixedResolutionBeforeDial": 1,
    "TestValidatedDialRechecksDNSForEachConnection": 2,
}
FILES = ["lib/netpolicy/netpolicy_test.go", "lib/netpolicy/netpolicy.go"]
HOOK = ROOT / "tools/netpolicy-scoreboard/hook_test.go"


def source():
    pinned = ROOT / references.MANIFEST["d2"]["checkout"]
    return {
        "commit": references.SOURCE_COMMIT,
        "files": {p: base.sha(pinned / p) for p in FILES},
        "hook": base.sha(HOOK),
    }


def instrument(text):
    for name in TESTS:
        old = f"func {name}(t *testing.T) {{"
        assert text.count(old) == 1
        text = text.replace(old, old + "\n d2rsSession(t)")
    # Only instrument the selected tests, leaving later transport tests intact.
    cutoff = text.index("func TestPublicOnlyClientChecksEveryRedirect")
    text = (
        text[:cutoff]
        .replace("isPublicAddress(", "d2rsPublic(t, ")
        .replace("newHTTPClient(", "d2rsClient(t, ")
        + text[cutoff:]
    )
    return text


def verify(capture):
    assert json.loads((capture / "source.json").read_text()) == source()
    assert json.loads((capture / "inventory.json").read_text()) == TESTS
    assert json.loads((capture / "hashes.json").read_text()) == base.hashes(capture)
    assert json.loads((capture / "run.json").read_text()) == {
        "exit": 0,
        "failure": None,
    }
    events = base.module("cli-scoreboard").events(capture / "go.jsonl")
    assert all(events.get(t) == "pass" for t in TESTS), events
    assert all(action != "fail" for action in events.values())
    cases = [json.loads(p.read_text()) for p in sorted((capture / "go").glob("*.json"))]
    assert len(cases) == len(TESTS) and {c["name"] for c in cases} == set(TESTS)
    for case in cases:
        assert case["test"] == case["name"]
        assert len(case["operations"]) == TESTS[case["name"]]
        for op in case["operations"]:
            assert events.get(op["test"]) == "pass"
        if case["name"] == "TestNonPublicAddressRanges":
            names = [op["test"] for op in case["operations"]]
            assert len(set(names)) == len(names)
            assert set(names) == {
                name for name in events if name.startswith(case["name"] + "/")
            }
    sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def capture_original(capture):
    pinned = references.verify_source()
    path = references.TRACE_CHECKOUT / FILES[0]
    original = (pinned / FILES[0]).read_bytes()
    helper = path.with_name("d2rs_netpolicy_test.go")
    assert path.read_bytes() == original and not helper.exists()
    (capture / "go").mkdir(parents=True, exist_ok=False)
    (capture / "tmp").mkdir()
    (capture / "source.json").write_text(json.dumps(source(), indent=2) + "\n")
    (capture / "inventory.json").write_text(json.dumps(TESTS, indent=2) + "\n")
    try:
        path.write_text(instrument(original.decode()))
        helper.write_bytes(HOOK.read_bytes())
        env = dict(
            os.environ,
            TMPDIR=str(capture / "tmp"),
            GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
            GOCACHE=str(ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod",
            GOPROXY="off",
            D2RS_NETPOLICY_DUMP=str(capture / "go"),
        )
        env.pop("TA", None)
        env.pop("TESTDATA_ACCEPT", None)
        result = base.module("cli-scoreboard").bounded_run(
            [
                "go",
                "test",
                "./lib/netpolicy",
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
        helper.unlink(missing_ok=True)
    (capture / "run.json").write_text(json.dumps(result) + "\n")
    assert result == {"exit": 0, "failure": None}, result
    (capture / "hashes.json").write_text(
        json.dumps(base.hashes(capture), indent=2) + "\n"
    )
    verify(capture)


def score(cases, actual):
    assert isinstance(actual, list) and len(actual) == len(cases)
    rows = []
    for case, got in zip(cases, actual, strict=True):
        expected = [op["got"] for op in case["operations"]]
        row = {"name": case["name"], "operations": len(expected)}
        row["outcome"] = (
            "unsupported"
            if isinstance(got, dict) and "unsupported" in got
            else "match"
            if expected == got
            else "mismatch"
        )
        if row["outcome"] == "mismatch":
            row["differences"] = base.module("language-scoreboard").differences(
                expected, got
            )
        rows.append(row)
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--rust", type=Path)
    parser.add_argument(
        "--baseline",
        action="store_true",
        help="old range source; missing old dial API remains unsupported",
    )
    parser.add_argument("--output", type=Path)
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
    request.write_text(
        json.dumps(
            {
                "baseline": args.baseline,
                "cases": [
                    {
                        "name": c["name"],
                        "operations": [op["request"] for op in c["operations"]],
                    }
                    for c in cases
                ],
            }
        )
    )
    assert args.rust and args.rust.is_file()
    digest = base.sha(args.rust)
    run = base.module("cli-scoreboard").bounded_run(
        [str(args.rust.resolve()), str(request.resolve())],
        os.environ.copy(),
        work,
        log,
        60,
    )
    if run == {"exit": 0, "failure": None}:
        rows = score(cases, json.loads(log.read_text()))
    else:
        rows = [
            {"name": c["name"], "outcome": run["failure"] or "rust-failed"}
            for c in cases
        ]
    assert base.sha(args.rust) == digest
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "runner_sha256": digest,
        "capture": str(capture.relative_to(ROOT)),
        "capture_hashes": base.hashes(capture),
        "baseline": args.baseline,
        "captured_functions": len(TESTS),
        "operations": sum(TESTS.values()),
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
