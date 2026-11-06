#!/usr/bin/env python3
"""Complete exact-time GIF prerequisite frames from pinned public capture tests.

Captures keep original Go assertions and callback behavior. Every emitted PNG,
including frames emitted before a callback failure, uses the shared raster gate.
Limits: 1 GiB capture, 128 MiB/file, 20 GiB free; 600 s per subprocess.
"""

from __future__ import annotations

import argparse
import base64
import collections
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
TESTS = [
    "TestAdmissionBeforeRendering",
    "TestNativeDeterministicCapture",
    "TestCaptureCallbackCancellation",
    "TestNativeTimelineAuthoredAnimation",
    "TestD2rsGIFFrameBoards",
]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


ISO = module("iso_frames", ROOT / "scripts/isometric-scoreboard.py")
RASTER = module("raster_frames", ROOT / "scripts/check-render-parity.py")


def capture(path):
    references.verify_source()
    package = references.TRACE_CHECKOUT / "d2renderers/d2isometric/d2isometricimg"
    source = package / "render.go"
    original = source.read_bytes()
    assert (
        original
        == (
            ISO.UPSTREAM / "d2renderers/d2isometric/d2isometricimg/render.go"
        ).read_bytes()
    )
    needle = b"func CaptureFrames("
    assert original.count(needle) == 1
    changed = original.replace(needle, b"func d2rsOriginalCaptureFrames(")
    hooks = [
        (package / ("d2rs_frames_" + name), ROOT / "tools/isometric-frame-probe" / name)
        for name in ["hook_test.go", "boards_test.go"]
    ]
    assert all(not p.exists() for p, _ in hooks)
    path.mkdir(parents=True, exist_ok=False)
    env = dict(
        os.environ,
        D2RS_FRAME_CAPTURE=str(path / "calls.jsonl"),
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
        assert code == 0, "original Go tests failed; inspect go-events.jsonl"
    finally:
        assert source.read_bytes() == changed
        source.write_bytes(original)
        for destination, _ in hooks:
            destination.unlink()
    events = [
        json.loads(line) for line in (path / "go-events.jsonl").read_text().splitlines()
    ]
    passed = {e["Test"] for e in events if e.get("Action") == "pass" and "Test" in e}
    assert set(TESTS) <= passed
    calls = [
        json.loads(line) for line in (path / "calls.jsonl").read_text().splitlines()
    ]
    assert calls and {c["test"] for c in calls} == set(TESTS)
    manifest = {
        "pin": references.SOURCE_COMMIT,
        "tests": TESTS,
        "passed": sorted(passed),
        "calls_sha256": references.digest(path / "calls.jsonl"),
        "events_sha256": references.digest(path / "go-events.jsonl"),
        "source_sha256": references.digest(source),
        "hooks": {str(p.relative_to(ROOT)): references.digest(p) for _, p in hooks},
        "budget": {"total": 1 << 30, "per_file": 128 << 20, "min_free": 20 << 30},
    }
    (path / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def materialize(result, directory):
    if not isinstance(result, dict) or set(result) != {"frames", "error"}:
        raise ValueError("invalid frame session")
    if not isinstance(result["error"], str):
        raise TypeError("invalid capture error")
    frames = result["frames"]
    if frames is None:
        frames = []
    if not isinstance(frames, list) or not result["error"] and not frames:
        raise ValueError("empty or invalid frame inventory")
    directory.mkdir()
    for i, frame in enumerate(frames):
        data = base64.b64decode(frame, validate=True)
        if not data or len(data) > 32 << 20:
            raise ValueError("invalid PNG size")
        (directory / f"{i:03}.png").write_bytes(data)
    if frames:
        inventory = RASTER.collect_png_outputs(directory.with_suffix(".png"))
        if len(inventory) != len(frames):
            raise ValueError("incomplete PNG inventory")
    return len(frames)


def compare(call, result, directory):
    want = call["want"]
    go = directory / "go"
    rust = directory / "rust"
    expected = materialize(want, go)
    try:
        actual = materialize(result, rust)
    except (ValueError, KeyError, TypeError, OSError):
        return "invalid-native-output"
    if actual != expected or want["error"] != result["error"]:
        return "mismatch"
    if (
        expected
        and not RASTER.compare_png_outputs(
            go.with_suffix(".png"), rust.with_suffix(".png")
        )["matched"]
    ):
        return "pixel-mismatch"
    return "match" if want == result else "byte-mismatch"


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
    assert manifest["pin"] == references.SOURCE_COMMIT
    assert references.digest(path / "calls.jsonl") == manifest["calls_sha256"]
    assert references.digest(path / "go-events.jsonl") == manifest["events_sha256"]
    assert (
        references.digest(
            ISO.UPSTREAM / "d2renderers/d2isometric/d2isometricimg/render.go"
        )
        == manifest["source_sha256"]
    )
    events = [
        json.loads(line) for line in (path / "go-events.jsonl").read_text().splitlines()
    ]
    passed = {e["Test"] for e in events if e.get("Action") == "pass" and "Test" in e}
    assert set(TESTS) <= passed
    calls = [
        json.loads(line) for line in (path / "calls.jsonl").read_text().splitlines()
    ]
    assert calls and {c["test"] for c in calls} == set(TESTS)
    assert len(calls) == 15, "pinned original/witness call inventory differs"
    work = args.output.with_suffix("")
    work.mkdir(parents=True, exist_ok=False)
    request = work / "requests.jsonl"
    request.write_text("".join(json.dumps(c["request"]) + "\n" for c in calls))
    result_path = work / "results.jsonl"
    # bounded_native owns group shutdown; a small stdin-forwarding command
    # leaves the runner synchronous and adds no normalization.
    command = [
        "/bin/zsh",
        "-c",
        'exec "$1" < "$2"',
        "frames",
        str(args.rust.resolve()),
        str(request.resolve()),
    ]
    with result_path.open("w") as log:
        code = ISO.bounded_native(
            command, log, lambda: ISO.direct_guard(work, (1 << 30) - ISO.size(path))
        )
    failure = ISO.direct_guard(work, (1 << 30) - ISO.size(path))
    assert not failure and code == 0, f"native capture failed: {code} {failure}"
    got = [json.loads(line) for line in result_path.read_text().splitlines()]
    assert len(got) == len(calls), "missing/extra native sessions"
    rows = []
    for i, (call, result) in enumerate(zip(calls, got)):
        directory = work / str(i)
        directory.mkdir()
        outcome = compare(call, result, directory)
        rows.append(
            {
                "call": i,
                "test": call["test"],
                "outcome": outcome,
                "expected_frames": len(call["want"]["frames"] or []),
                "actual_frames": len(result["frames"] or []),
                "expected_error": call["want"]["error"],
                "actual_error": result["error"],
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
