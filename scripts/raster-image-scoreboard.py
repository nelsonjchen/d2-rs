#!/usr/bin/env python3
"""Exact PNG bytes at isometric rasterURL's DecodeFirst/png.Encode boundary."""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
TESTS = {
    "TestJPEGEXIFOrientationAllTransforms": 9,
    "TestJPEGMalformedEXIFIsIgnored": 8,
    "TestAnimatedGIFFirstFrame": 1,
    "TestAPNGFirstAnimationFrameExcludesDefaultImage": 1,
    "TestAPNGAllowsDefaultFrameControlBeforeAnimationControl": 1,
    "TestAnimatedWebPFirstFrame": 1,
    "TestStaticWebPIgnoresStrayANIMWithoutAnimationFlag": 1,
    "TestAnimatedWebPSkipsUnknownPaddedFrameChunk": 1,
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def capture_go(capture):
    source = references.verify_source()
    trace = references.TRACE_CHECKOUT
    assert (
        subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=trace, text=True
        ).strip()
        == references.SOURCE_COMMIT
    )
    package = trace / "internal/rasterimage"
    path = package / "firstframe_test.go"
    original = path.read_text()
    assert original == (source / "internal/rasterimage/firstframe_test.go").read_text()
    hook = package / "d2rs_image_hook_test.go"
    assert not hook.exists()
    try:
        text = original
        for name in TESTS:
            match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
            assert match, name
            body, count = re.subn(r"\bDecodeFirst\(", "d2rsDecodeFirst(t, ", match[0])
            assert count, name
            text = text[: match.start()] + body + text[match.end() :]
        path.write_text(text)
        hook.write_bytes(
            (ROOT / "tools/raster-image-scoreboard/hook_test.go").read_bytes()
        )
        env = os.environ.copy()
        env.update(
            D2RS_IMAGE_CAPTURE=str(capture),
            GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
            GOCACHE=str(ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod",
            GOPROXY="off",
        )
        with (capture / "events.jsonl").open("w") as log:
            result = subprocess.run(
                [
                    "go",
                    "test",
                    "-json",
                    "./internal/rasterimage",
                    "-run",
                    "^(" + "|".join(TESTS) + ")$",
                    "-count=1",
                ],
                cwd=trace,
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
                timeout=600,
                check=False,
            )
        events = []
        for line in (capture / "events.jsonl").read_text().splitlines():
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                pass
        outcomes = {
            e["Test"]: e["Action"]
            for e in events
            if "Test" in e and e.get("Action") in ("pass", "fail", "skip")
        }
        assert result.returncode == 0 and all(
            outcomes.get(n) == "pass" for n in TESTS
        ), outcomes
        cases = sorted((capture / "go").rglob("case.json"))
        counts = collections.Counter(
            json.loads(p.read_text())["test"].split("/")[0] for p in cases
        )
        assert dict(counts) == TESTS, counts
        for case in cases:
            assert outcomes[json.loads(case.read_text())["test"]] == "pass"
        manifest = {
            "source_commit": references.SOURCE_COMMIT,
            "outcomes": outcomes,
            "source_sha256": hashlib.sha256(original.encode()).hexdigest(),
            "hook_sha256": digest(hook),
            "files": {
                str(p.relative_to(capture)): digest(p)
                for p in capture.rglob("*")
                if p.is_file()
            },
        }
        (capture / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n")
    finally:
        path.write_text(original)
        hook.unlink(missing_ok=True)


def compare_case(expected, actual, native_exit=0):
    return (
        native_exit == 0
        and actual.is_file()
        and actual.read_bytes() == expected.read_bytes()
        and len(list(actual.parent.iterdir())) == 1
    )


def score(capture, rust, output):
    manifest = json.loads((capture / "manifest.json").read_text())
    assert manifest["source_commit"] == references.SOURCE_COMMIT
    actual_files = {
        str(p.relative_to(capture))
        for p in capture.rglob("*")
        if p.is_file() and p.name != "manifest.json"
    }
    assert actual_files == set(manifest["files"]), "source inventory changed"
    for name, expected in manifest["files"].items():
        assert digest(capture / name) == expected, name
    cases = sorted((capture / "go").rglob("case.json"))
    assert len(cases) == sum(TESTS.values())
    expected_go_files = set()
    for case in cases:
        record = json.loads(case.read_text())
        assert manifest["outcomes"].get(record["test"]) == "pass"
        expected = [
            case.parent / ("expected." + ext)
            for ext in ("png", "err")
            if (case.parent / ("expected." + ext)).is_file()
        ]
        assert len(expected) == 1 and expected[0].stat().st_size > 0
        assert (case.parent / "input.bin").stat().st_size > 0
        expected_go_files.update([case, case.parent / "input.bin", expected[0]])
    assert {p for p in (capture / "go").rglob("*") if p.is_file()} == expected_go_files
    counts = collections.Counter(
        json.loads(p.read_text())["test"].split("/")[0] for p in cases
    )
    assert dict(counts) == TESTS
    native = output.with_suffix("")
    assert not native.exists(), "native output path must be fresh"
    native.mkdir(parents=True)
    requests, expected_files = [], set()
    for case in cases:
        request = json.loads(case.read_text())
        directory = native / case.parent.relative_to(capture / "go")
        request.update(input=str(case.parent / "input.bin"), out=str(directory))
        requests.append(request)
        assert (case.parent / "expected.png").is_file() != (
            case.parent / "expected.err"
        ).is_file()
        expected_files.add(
            directory
            / (
                "actual.png"
                if (case.parent / "expected.png").is_file()
                else "actual.err"
            )
        )
    result = subprocess.run(
        [str(rust)],
        input="\n".join(json.dumps(r) for r in requests) + "\n",
        text=True,
        capture_output=True,
        timeout=600,
        check=False,
    )
    (native / "rust.log").write_text(result.stdout + result.stderr)
    rows = []
    for case, request in zip(cases, requests):
        suffix = "png" if (case.parent / "expected.png").is_file() else "err"
        actual = Path(request["out"]) / ("actual." + suffix)
        expected = case.parent / ("expected." + suffix)
        matched = compare_case(expected, actual, result.returncode)
        rows.append(
            {
                "board": str(case.parent.relative_to(capture / "go")),
                "outcome": "match" if matched else "mismatch",
                "kind": "png-bytes" if suffix == "png" else "error",
                "expected_sha256": digest(expected),
                "actual_sha256": digest(actual) if actual.is_file() else None,
            }
        )
    extras = (
        {p for p in native.rglob("*") if p.is_file()}
        - expected_files
        - {native / "rust.log"}
    )
    # A wrong-type outcome is already a mismatch; unrelated extra artifacts fail too.
    known_outcomes = {
        Path(r["out"]) / ("actual." + suffix)
        for r in requests
        for suffix in ("png", "err")
    }
    for path in sorted(extras - known_outcomes):
        rows.append(
            {
                "board": str(path.relative_to(native)),
                "outcome": "unexpected-native-file",
            }
        )
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "rust": str(rust),
        "rust_sha256": digest(rust),
        "source_capture": str(capture),
        "native_capture": str(native),
        "native_exit": result.returncode,
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows,
    }
    output.write_text(json.dumps(report, indent=1) + "\n")
    print(report["summary"])
    return report


def delta(before, after):
    left = {r["board"]: r for r in before["rows"]}
    right = {r["board"]: r for r in after["rows"]}
    assert len(left) == len(before["rows"]) and len(right) == len(after["rows"])
    assert (
        before["source_commit"] == after["source_commit"]
        and left.keys() == right.keys()
    )
    regressions = [
        k
        for k in left
        if left[k]["outcome"] == "match" and right[k]["outcome"] != "match"
    ]
    improvements = [
        k
        for k in left
        if left[k]["outcome"] != "match" and right[k]["outcome"] == "match"
    ]
    return {
        "before": before["summary"],
        "after": after["summary"],
        "regressions": regressions,
        "improvements": improvements,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--before", type=Path)
    args = parser.parse_args()
    if not args.reuse:
        args.capture.mkdir(parents=True, exist_ok=False)
        capture_go(args.capture.resolve())
    after = score(args.capture.resolve(), args.rust.resolve(), args.output.resolve())
    if args.before:
        comparison = delta(json.loads(args.before.read_text()), after)
        args.output.with_suffix(".delta.json").write_text(
            json.dumps(comparison, indent=1) + "\n"
        )
        assert not comparison["regressions"], comparison


if __name__ == "__main__":
    main()
