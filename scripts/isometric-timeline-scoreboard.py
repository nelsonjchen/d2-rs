#!/usr/bin/env python3
"""Pinned ordered-board timeline observations, with complete decoded frames and timing.

Original Go assertions stay active. Each run is limited to 1 GiB total,
128 MiB/file, 20 GiB free and 600 seconds per subprocess.
"""

from __future__ import annotations

import argparse
import collections
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
TESTS = [
    "TestNativeTimelineBoardOrderPaletteAndDelay",
    "TestNativeTimelineAuthoredAnimation",
    "TestNativeTimelineAdmission",
    "TestTimelineUsesOneFrameForBoardsWithDifferentExtents",
]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


ISO = module("iso_gif", ROOT / "scripts/isometric-scoreboard.py")
GIF = module("timeline_gif_gate", ROOT / "scripts/isometric-gif-scoreboard.py")
compare = GIF.compare


def capture(path):
    references.verify_source()
    package = references.TRACE_CHECKOUT / "d2renderers/d2isometric/d2isometricimg"
    source = package / "timeline.go"
    original = source.read_bytes()
    assert (
        original
        == (
            ISO.UPSTREAM / "d2renderers/d2isometric/d2isometricimg/timeline.go"
        ).read_bytes()
    )
    needle = b"func RenderTimeline("
    assert original.count(needle) == 1
    changed = original.replace(needle, b"func d2rsOriginalRenderTimeline(")
    hooks = [
        (package / ("d2rs_timeline_" + name), ROOT / "tools/isometric-timeline-probe" / name)
        for name in ["hook_test.go"]
    ]
    assert all(not p.exists() for p, _ in hooks)
    path.mkdir(parents=True, exist_ok=False)
    (path / "tmp").mkdir()
    env = dict(
        os.environ,
        D2RS_TIMELINE_CAPTURE=str(path / "calls.jsonl"),
        TMPDIR=str(path / "tmp"),
        GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
        GOCACHE=str(ROOT / ".reference/go-build"),
        GOFLAGS="-mod=mod",
        GOPROXY="off",
    )
    try:
        for destination, template in hooks:
            destination.write_bytes(template.read_bytes())
        source.write_bytes(changed)
        with (path / "go-events.jsonl").open("w") as log:
            command = [
                "env",
                *[f"{k}={v}" for k, v in env.items() if os.environ.get(k) != v],
                "go",
                "-C",
                str(references.TRACE_CHECKOUT),
                "test",
                "./d2renderers/d2isometric/d2isometricimg",
                "-json",
                "-count=1",
                "-run",
                "^(" + "|".join(TESTS) + ")$",
            ]
            code = ISO.bounded_native(
                command, log, lambda: ISO.direct_guard(path, 1 << 30)
            )
        assert code == 0, "original Go assertions failed; see go-events.jsonl"
    finally:
        assert source.read_bytes() == changed
        source.write_bytes(original)
        for destination, _ in hooks:
            destination.unlink()
    passed = passed_tests(path)
    assert set(TESTS) <= passed
    calls = [
        json.loads(line) for line in (path / "calls.jsonl").read_text().splitlines()
    ]
    assert calls and {c["test"] for c in calls} == set(TESTS)
    manifest = {
        "pin": references.SOURCE_COMMIT,
        "tests": TESTS,
        "calls": len(calls),
        "passed": sorted(passed),
        "calls_sha256": references.digest(path / "calls.jsonl"),
        "events_sha256": references.digest(path / "go-events.jsonl"),
        "source_sha256": references.digest(source),
        "hooks": {str(p.relative_to(ROOT)): references.digest(p) for _, p in hooks},
        "budget": {"total": 1 << 30, "per_file": 128 << 20, "min_free": 20 << 30},
    }
    (path / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def passed_tests(path):
    events = [
        json.loads(line) for line in (path / "go-events.jsonl").read_text().splitlines()
    ]
    return {e["Test"] for e in events if e.get("Action") == "pass" and "Test" in e}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    path = args.capture.resolve()
    if not args.reuse:
        capture(path)
    references.verify_source()
    manifest = json.loads((path / "manifest.json").read_text())
    assert manifest["pin"] == references.SOURCE_COMMIT and manifest["tests"] == TESTS
    assert references.digest(path / "calls.jsonl") == manifest["calls_sha256"]
    assert references.digest(path / "go-events.jsonl") == manifest["events_sha256"]
    assert set(TESTS) <= passed_tests(path)
    assert (
        references.digest(
            ISO.UPSTREAM / "d2renderers/d2isometric/d2isometricimg/timeline.go"
        )
        == manifest["source_sha256"]
    )
    calls = [
        json.loads(line) for line in (path / "calls.jsonl").read_text().splitlines()
    ]
    assert len(calls) == manifest["calls"] > 0 and {c["test"] for c in calls} == set(
        TESTS
    )
    work = args.output.with_suffix("")
    work.mkdir(parents=True, exist_ok=False)
    request = work / "requests.jsonl"
    request.write_text("".join(json.dumps(c["request"]) + "\n" for c in calls))
    result_path = work / "results.jsonl"
    command = [
        "/bin/zsh",
        "-c",
        'exec "$1" < "$2"',
        "gifs",
        str(args.rust.resolve()),
        str(request.resolve()),
    ]
    with result_path.open("w") as log:
        code = ISO.bounded_native(
            command, log, lambda: ISO.direct_guard(work, (1 << 30) - ISO.size(path))
        )
    assert code == 0 and not ISO.direct_guard(work, (1 << 30) - ISO.size(path)), (
        "native capture failed"
    )
    results = [json.loads(line) for line in result_path.read_text().splitlines()]
    assert len(calls) == len(results), "missing/extra native results"
    rows = []
    for i, (call, got) in enumerate(zip(calls, results)):
        directory = work / str(i)
        directory.mkdir()
        outcome, frames = compare(call["want"], got, directory)
        rows.append(
            {
                "call": i,
                "test": call["test"],
                "outcome": outcome,
                "frames": frames,
                "expected_error": call["want"]["error"],
                "actual_error": got.get("error", ""),
            }
        )
    assert not ISO.direct_guard(work, (1 << 30) - ISO.size(path))
    report = {
        "pin": references.SOURCE_COMMIT,
        "capture": str(path),
        "rust": str(args.rust.resolve()),
        "rust_sha256": references.digest(args.rust),
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
