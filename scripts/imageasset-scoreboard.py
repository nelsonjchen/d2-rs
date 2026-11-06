#!/usr/bin/env python3
"""Original pinned imageasset acquisition calls, including complete resource bytes."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import signal
import subprocess
import time
from collections import Counter
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
TESTS = {
    "TestResolveLocalRelativeAbsoluteAndOwnedBytes": 2,
    "TestResourceDataURIContext": 1,
    "TestResolveRasterDataURIFormats/png": 1,
    "TestResolveRasterDataURIFormats/jpeg": 1,
    "TestResolveRasterDataURIFormats/gif": 1,
    "TestResolveRasterDataURIFormats/webp": 1,
    **{
        f"TestResolverLimits/{name}": count
        for name, count in [
            ("fetched", 1),
            ("encoded", 1),
            ("decompressed", 1),
            ("width", 1),
            ("height", 1),
            ("pixels", 1),
            ("single_decoded_footprint", 1),
            ("cumulative", 2),
            ("inclusive", 1),
        ]
    },
}
SVG_TESTS = {
    "TestResolveStrictRawSVGDoesNotFetchNestedResources": 1,
    "TestResolveCanonicalSVGDoctypePreservesBytesAndEntitiesStayDisabled": 3,
    "TestResolveISO88591SVGPreservesSourceBytesAndRejectsDTD": 4,
}
SVG_CODEC_TESTS = {
    f"TestCompressedDeclaredSVGCodecsAndLimits/{name}": 1
    for name in [
        "gzip",
        "zlib",
        "raw-deflate",
        "brotli",
        "raw-deflate-leading-xml-byte",
        "local_svgz",
        "bomb",
    ]
}
SUITES = {"raster": TESTS, "svg": SVG_TESTS, "svg-codecs": SVG_CODEC_TESTS}
HTTP_TESTS = {
    "TestResolveHTTPRedirectAndContentSniffing": 2,
    **{
        f"TestResolveHTTPContentEncodings/{name}": 1
        for name in [
            "gzip",
            "x-gzip",
            "deflate-zlib",
            "deflate-raw",
            "brotli",
            "stacked",
        ]
    },
    "TestHTTPDecompressionAndEncodedLimits": 2,
    "TestHTTPDecompressionAndEncodedLimits/streamed_fetched_bytes": 1,
}
SUITES["http"] = HTTP_TESTS
HTTP_HEADER_TESTS = {
    "TestHTTPFragmentsShareCanonicalSnapshot": 2,
    **{
        f"TestHTTPHeaderBoundsAndTokenParsing//{name}": 1
        for name in [
            "encoding-bytes",
            "empty-encoding",
            "encoding-layers",
            "content-type-bytes",
            "multiple-content-types",
        ]
    },
}
SUITES["http-headers"] = HTTP_HEADER_TESTS
CACHE_TESTS = {
    "TestMemoryCacheBoundedEviction": 7,
    "TestSharedCacheRevalidatesResolverLimits": 2,
    "TestSharedCacheRevalidatesSVGByteLimit": 2,
}
LOCAL_CACHE_TESTS = {
    "TestOptionalCacheAndCumulativeCharging": 4,
    "TestSharedCacheSkipsInlineDataButRetainsLocalResources": 5,
    "TestLocalCacheKeysIncludeResolvedBaseDirectory": 2,
}
NAMESPACE_TESTS = {"TestSharedCacheRequiresAndIsolatesNamespace": 3}
SUITES.update(
    {
        "cache": CACHE_TESTS,
        "cache-local": LOCAL_CACHE_TESTS,
        "cache-namespace": NAMESPACE_TESTS,
    }
)


def uses_http_hook(tests):
    return tests in [
        HTTP_TESTS,
        HTTP_HEADER_TESTS,
        CACHE_TESTS,
        LOCAL_CACHE_TESTS,
        NAMESPACE_TESTS,
    ]


def suite_name(tests):
    return next(name for name, inventory in SUITES.items() if inventory == tests)


def bounded_run(command, capture, **kwargs):
    """Bound active source/native output; budget exhaustion never passes."""
    process = subprocess.Popen(command, start_new_session=True, **kwargs)
    deadline = time.monotonic() + 600
    try:
        while process.poll() is None:
            sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
            volume = os.statvfs(ROOT)
            assert sum(sizes) <= 1 << 30 and max(sizes, default=0) <= 128 << 20, (
                "capture budget exceeded"
            )
            assert volume.f_bavail * volume.f_frsize >= 20 << 30, (
                "free-space budget exceeded"
            )
            assert time.monotonic() < deadline, "capture timeout"
            time.sleep(0.1)
        sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
        assert sum(sizes) <= 1 << 30 and max(sizes, default=0) <= 128 << 20, (
            "capture budget exceeded"
        )
        return process.returncode
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def value_sha(value):
    return hashlib.sha256(
        json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def compare(expected, actual):
    spec = importlib.util.spec_from_file_location(
        "language_scoreboard", ROOT / "scripts/language-scoreboard.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return not module.differences(expected, actual)


def delta(before, after):
    left = {r["case"]: r for r in before["rows"]}
    right = {r["case"]: r for r in after["rows"]}
    assert len(left) == len(before["rows"]) and len(right) == len(after["rows"]), (
        "duplicate cases"
    )
    assert left.keys() == right.keys(), "changed input inventory"
    for case in left:
        for field in ["input_sha256", "expected_sha256"]:
            assert left[case][field] == right[case][field], (case, field)
    return {
        "before_matches": sum(r["outcome"] == "match" for r in left.values()),
        "after_matches": sum(r["outcome"] == "match" for r in right.values()),
        "improvements": [
            c
            for c in left
            if left[c]["outcome"] != "match" and right[c]["outcome"] == "match"
        ],
        "regressions": [
            c
            for c in left
            if left[c]["outcome"] == "match" and right[c]["outcome"] != "match"
        ],
    }


def capture_go(capture, tests=TESTS):
    source = references.verify_source()
    assert (
        subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=references.TRACE_CHECKOUT, text=True
        ).strip()
        == references.SOURCE_COMMIT
    )
    package = references.TRACE_CHECKOUT / "lib/imageasset"
    filename = (
        "remediation_test.go"
        if tests in [SVG_CODEC_TESTS, HTTP_HEADER_TESTS, NAMESPACE_TESTS]
        else "security_test.go"
        if tests == CACHE_TESTS
        else "imageasset_test.go"
    )
    path = package / filename
    original = path.read_bytes()
    assert original == (source / "lib/imageasset" / filename).read_bytes()
    hook = package / "d2rs_capture_test.go"
    assert not hook.exists()
    text = original.decode()
    resolve_path = package / "resolve.go"
    resolve_original = resolve_path.read_bytes()
    assert resolve_original == (source / "lib/imageasset/resolve.go").read_bytes()
    names = sorted({name.split("/")[0] for name in tests})
    inventory = re.findall(r"^func (Test\w+)\(", text, re.MULTILINE)
    all_files = {
        str(p.relative_to(source)): sha(p)
        for p in (source / "lib/imageasset").glob("*_test.go")
    }
    try:
        for name in names:
            match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
            assert match, name
            body, count = re.subn(
                r"\b([a-zA-Z_]\w*)\.Resolve\(",
                r"d2rsResolve(t, \1, ",
                match[0],
            )
            if tests in [SVG_CODEC_TESTS, HTTP_HEADER_TESTS, NAMESPACE_TESTS]:
                body, helper_count = re.subn(
                    r"\bmustResolve\(", "d2rsMustResolve(", body
                )
                assert count + helper_count, name
            else:
                assert count, name
            text = text[: match.start()] + body + text[match.end() :]
        path.write_text(text)
        hook.write_bytes(
            (ROOT / "tools/imageasset-scoreboard/hook_test.go").read_bytes()
        )
        if uses_http_hook(tests):
            resolve_text = resolve_original.decode()
            assert resolve_text.count("r.client.Do(request)") == 1
            resolve_path.write_text(
                resolve_text.replace("r.client.Do(request)", "d2rsHTTPDo(r, request)")
            )
        env = dict(
            os.environ,
            D2RS_IMAGEASSET_CAPTURE=str(capture),
            TMPDIR=str(capture),
            GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
            GOCACHE=str(ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod",
            GOPROXY="off",
        )
        with (capture / "events.jsonl").open("w") as log:
            result = bounded_run(
                [
                    "go",
                    "test",
                    "-json",
                    ".",
                    "-run",
                    "^(" + "|".join(names) + ")$",
                    "-count=1",
                ],
                capture,
                cwd=package,
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
            )
        events = [
            json.loads(line)
            for line in (capture / "events.jsonl").read_text().splitlines()
            if line.startswith("{")
        ]
        outcomes = {
            e["Test"]: e["Action"]
            for e in events
            if "Test" in e and e["Action"] in ("pass", "fail", "skip")
        }
        assert result == 0 and all(outcomes.get(name) == "pass" for name in tests), (
            outcomes
        )
        calls = [
            json.loads(line)
            for line in (capture / "calls.jsonl").read_text().splitlines()
        ]
        assert Counter(row["request"]["test"] for row in calls) == tests
        manifest = {
            "source_commit": references.SOURCE_COMMIT,
            "suite": suite_name(tests),
            "primary_file": filename,
            "test_files": all_files,
            "uncovered_in_primary_file": sorted(set(inventory) - set(names)),
            "outcomes": outcomes,
            "hook_sha256": sha(hook),
            "files": {p.name: sha(p) for p in capture.iterdir() if p.is_file()},
        }
        (capture / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n")
    finally:
        path.write_bytes(original)
        if uses_http_hook(tests):
            resolve_path.write_bytes(resolve_original)
        hook.unlink(missing_ok=True)


def verify(capture, tests=TESTS):
    manifest = json.loads((capture / "manifest.json").read_text())
    assert manifest["source_commit"] == references.SOURCE_COMMIT
    assert manifest.get("suite", "raster") == suite_name(tests)
    assert {p.name for p in capture.iterdir()} == set(manifest["files"]) | {
        "manifest.json"
    }
    for name, digest in manifest["files"].items():
        assert sha(capture / name) == digest, name
    rows = [
        json.loads(line) for line in (capture / "calls.jsonl").read_text().splitlines()
    ]
    assert Counter(row["request"]["test"] for row in rows) == tests
    assert all(manifest["outcomes"].get(name) == "pass" for name in tests)
    source = references.verify_source()
    assert set(manifest["test_files"]) == {
        str(p.relative_to(source))
        for p in (source / "lib/imageasset").glob("*_test.go")
    }, "changed test file inventory"
    for path, digest in manifest["test_files"].items():
        assert sha(source / path) == digest, "changed original test source"
    return manifest, rows


def score(capture, runner, output, baseline=False, tests=TESTS):
    manifest, rows = verify(capture, tests)
    pristine = (ROOT / references.MANIFEST["d2"]["checkout"]).resolve()
    replay = output.with_suffix(".replay")
    replay.mkdir()
    native = replay / "native.jsonl"
    if baseline:
        results = [
            {"unsupported": "native imageasset resolver is absent"} for _ in rows
        ]
        native.write_text("".join(json.dumps(r) + "\n" for r in results))
        exit_code = 0
    else:
        requests = replay / "requests.jsonl"
        requests.write_text("".join(json.dumps(r["request"]) + "\n" for r in rows))
        with (
            requests.open() as source,
            native.open("w") as sink,
            (replay / "stderr.log").open("w") as log,
        ):
            exit_code = bounded_run(
                [str(runner.resolve())], replay, stdin=source, stdout=sink, stderr=log
            )
        results = [json.loads(line) for line in native.read_text().splitlines()]
    assert len(results) == len(rows), "missing native outputs"
    scored = [
        {
            "case": f"{row['request']['test']}#{i}",
            "outcome": "match"
            if exit_code == 0 and compare(row["expected"], actual)
            else "unsupported"
            if baseline
            else "mismatch",
            "input_sha256": value_sha(row["request"]),
            "expected_sha256": value_sha(row["expected"]),
            "expected": row["expected"],
            "actual": actual,
        }
        for i, (row, actual) in enumerate(zip(rows, results, strict=True))
    ]
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "capture": str(capture),
        "runner": None if baseline else str(runner.resolve()),
        "runner_sha256": None if baseline else sha(runner),
        "uncovered_in_primary_file": manifest["uncovered_in_primary_file"],
        "source_test_inventory": manifest["test_files"],
        "summary": dict(Counter(row["outcome"] for row in scored)),
        "rows": scored,
        "uncaptured_original_calls": [
            "TestCompressedDeclaredSVGCodecsAndLimits/canceled: direct internal expandDeclaredSVG call; Go assertion ran, native call not captured"
        ]
        if tests == SVG_CODEC_TESTS
        else [
            "TestHTTPHeaderBoundsAndTokenParsing: direct canceled parseContentEncodings call and default transport MaxResponseHeaderBytes assertion ran in Go; not captured natively"
        ]
        if tests == HTTP_HEADER_TESTS
        else [
            "TestSharedCacheRequiresAndIsolatesNamespace: direct New call rejecting an empty namespace runs in Go; Resolve calls are captured, native constructor rejection is independently unit tested"
        ]
        if tests == NAMESPACE_TESTS
        else [],
    }
    covered = {name.split("/")[0] for name in tests}
    report["uncovered_original_functions"] = [
        f"{path}::{name}"
        for path in manifest["test_files"]
        for name in re.findall(
            r"^func (Test\w+)\(", (pristine / path).read_text(), re.MULTILINE
        )
        if path
        != "lib/imageasset/" + manifest.get("primary_file", "imageasset_test.go")
        or name not in covered
    ]
    output.write_text(json.dumps(report, indent=1) + "\n")
    print(report["summary"])
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--suite", choices=list(SUITES), default="raster")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runner", type=Path)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--baseline", action="store_true")
    parser.add_argument("--before", type=Path)
    parser.add_argument("--delta", type=Path)
    args = parser.parse_args()
    tests = SUITES[args.suite]
    capture = args.capture.resolve()
    if not args.reuse:
        assert not capture.exists(), "use a fresh capture"
        capture.mkdir(parents=True)
        capture_go(capture, tests)
    report = score(capture, args.runner, args.output, args.baseline, tests)
    if args.before:
        changes = delta(json.loads(args.before.read_text()), report)
        assert args.delta
        args.delta.write_text(json.dumps(changes, indent=1) + "\n")
        assert not changes["regressions"], changes


if __name__ == "__main__":
    main()
