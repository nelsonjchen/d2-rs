#!/usr/bin/env python3
"""PNG goldens for pinned D2 isometric: require reproduced Go pixels first.

Invoked by isometric-scoreboard.py --tier png. The mesh runner receives Go's
ordinary raster meshes and complete textures. --native additionally scores the
end-to-end native diagram probe. No SVG mesh is substituted for a raster mesh.
"""
from __future__ import annotations

import argparse
import collections
import contextlib
import importlib.util
import json
import os
import subprocess
import zlib
from pathlib import Path

import references

ROOT = references.ROOT

def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result

ISO = module("iso_scoreboard", "isometric-scoreboard.py")
RASTER = module("render_parity", "check-render-parity.py")


def inventory():
    source = (ROOT / references.MANIFEST["d2"]["checkout"]).resolve()
    png = source / "e2etests/png/testdata"
    goldens = sorted(png.rglob("isometric*.exp.png"))
    if not goldens:
        raise ValueError("empty PNG golden inventory")
    return source, png, goldens


@contextlib.contextmanager
def hooks():
    package = references.TRACE_CHECKOUT / "d2renderers/d2isometric/d2isometricimg"
    installed = []
    try:
        for name in ("png_hook_test.go",):
            target = package / ("d2rs_" + name)
            data = (ROOT / "tools/isometric-probe" / name).read_bytes()
            if target.exists():
                if target.read_bytes() != data:
                    raise ValueError(f"concurrent differing hook: {target}")
            else:
                target.write_bytes(data)
                installed.append((target, data))
        yield
    finally:
        for target, data in reversed(installed):
            if target.read_bytes() != data:
                raise ValueError(f"hook changed during capture: {target}")
            target.unlink()


def strict_pixels(expected, actual):
    # Reuse both prescribed gates; exact pixel equality is additionally required.
    result = RASTER.compare_png_outputs(expected, actual)
    try:
        left, right = RASTER.collect_png_outputs(expected), RASTER.collect_png_outputs(actual)
        result["matched"] = bool(left) and left == right
    except (OSError, ValueError, zlib.error) as error:
        result.update(matched=False, error=str(error))
    return result


def score(capture, attempt, runner, native, go_exit, failure):
    _, png, goldens = inventory()
    expected = {g.relative_to(png).as_posix().removesuffix(".exp.png"): g for g in goldens}
    actual = {p.relative_to(capture / "go").as_posix().removesuffix(".png")
              for p in (capture / "go").rglob("*.png")}
    rows = []
    for name in sorted(set(expected) | actual):
        row = {"board": name}
        golden = expected.get(name)
        source = capture / "go" / (name + ".png")
        if golden is None:
            row["outcome"] = "go-extra"
        elif go_exit or failure:
            row["outcome"] = "incomplete-run"
        elif not source.is_file():
            row["outcome"] = "go-missing"
        else:
            row["golden_sha256"] = references.digest(golden)
            row["go_sha256"] = references.digest(source)
            row["qualification"] = strict_pixels(golden, source)
            if not row["qualification"]["matched"]:
                row["outcome"] = "go-unreproduced"
            elif not runner.is_file():
                row["outcome"] = "rust-unavailable"
            else:
                output = attempt / "rust" / (name + ".png")
                output.parent.mkdir(parents=True, exist_ok=True)
                mesh = source.with_suffix(".mesh.json")
                with (attempt / "rust.log").open("a") as log:
                    try:
                        code = ISO.bounded_native([str(runner.resolve()), str(mesh), str(output)], log,
                            lambda: ISO.direct_guard(capture, 1 << 30), timeout=180)
                    except (ISO.CaptureLimit, subprocess.TimeoutExpired) as error:
                        code = -1
                        row["failure"] = str(error)
                row["mesh_sha256"] = references.digest(mesh) if mesh.is_file() else None
                row["comparison"] = strict_pixels(source, output)
                row["outcome"] = "match" if code == 0 and row["comparison"]["matched"] else "mismatch"
                if output.is_file():
                    row["rust_sha256"] = references.digest(output)
                if output.with_suffix(".png.err").is_file():
                    row["rust_error"] = output.with_suffix(".png.err").read_text()
                if native:
                    native_output = attempt / "native" / (name + ".png")
                    row["native_comparison"] = strict_pixels(source, native_output)
                    row["native_outcome"] = "match" if row["native_comparison"]["matched"] else "mismatch"
        rows.append(row)
    if native:
        native_root = attempt / "native"
        for path in native_root.rglob("*"):
            if path.is_file():
                relative = path.relative_to(native_root).as_posix()
                name = relative.removesuffix(".png")
                if path.suffix != ".png" or name not in expected:
                    rows.append({"board": relative, "outcome": "native-extra", "native_outcome": "unexpected-output"})
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tier", choices=["png"], required=True)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/examples/isometric-raster")
    parser.add_argument("--native", type=Path)
    parser.add_argument("--reuse", action="store_true")
    args = parser.parse_args()
    references.verify_source()
    source, png, goldens = inventory()
    capture = args.capture.resolve()
    if not args.reuse:
        if capture.exists():
            parser.error("use a fresh capture directory")
        capture.mkdir(parents=True)
        fixtures = sorted({g.parent.relative_to(png).as_posix() for g in goldens})
        requests = [{"Board": str(source / "e2etests/testdata" / fixture / "board.exp.json"),
                     "Out": str(capture / "go" / fixture), "Assets": str(capture / "textures")}
                    for fixture in fixtures]
        request_path = capture / "requests.jsonl"
        request_path.write_text("".join(json.dumps(r) + "\n" for r in requests))
        env = dict(os.environ, D2RS_ISOMETRIC_PNG=str(request_path),
            GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"), GOCACHE=str(ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod", GOPROXY="off", GOTOOLCHAIN=references.MANIFEST["d2"]["go_toolchain"])
        failure, code = None, -1
        with hooks(), (capture / "go.log").open("w") as log:
            # cwd is explicit; caches are absolute and shared across all lanes.
            command = ["go", "-C", str(references.TRACE_CHECKOUT), "test", "./d2renderers/d2isometric/d2isometricimg",
                       "-run", "^TestD2rsPNGCapture$", "-count=1", "-timeout=15m"]
            # bounded_native inherits environment, restored even on capture failure.
            old = {key: os.environ.get(key) for key in env if os.environ.get(key) != env[key]}
            try:
                os.environ.update(env)
                code = ISO.bounded_native(command, log, lambda: ISO.direct_guard(capture, 1 << 30), timeout=1000)
            except (ISO.CaptureLimit, subprocess.TimeoutExpired) as error:
                failure = str(error)
            finally:
                for key, value in old.items():
                    if value is None:
                        os.environ.pop(key, None)
                    else:
                        os.environ[key] = value
        manifest = {"source_commit": references.SOURCE_COMMIT, "go_exit": code, "failure": failure,
                    "hooks": {name: references.digest(ROOT / "tools/isometric-probe" / name)
                              for name in ("png_hook_test.go",)},
                    "goldens": {str(g): references.digest(g) for g in goldens},
                    "inputs": {r["Board"]: references.digest(Path(r["Board"])) for r in requests},
                    "files": {str(p.relative_to(capture)): references.digest(p) for p in capture.rglob("*") if p.is_file()}}
        (capture / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n")
    else:
        manifest = json.loads((capture / "manifest.json").read_text())
        if manifest["source_commit"] != references.SOURCE_COMMIT:
            raise ValueError("stale source pin")
        for path, digest in manifest["inputs"].items():
            if references.digest(Path(path)) != digest:
                raise ValueError(f"changed input {path}")
        for path, digest in manifest["files"].items():
            if references.digest(capture / path) != digest:
                raise ValueError(f"changed capture {path}")
    attempt = capture / args.output.stem
    if attempt.exists() or args.output.exists():
        parser.error("use a fresh result path; replay never overwrites prior evidence")
    attempt.mkdir()
    if args.native:
        native_rows = [{"board": p, "out": str(attempt / "native" / Path(p).parent.relative_to(source / "e2etests/testdata")), "mode": "png"}
                       for p in manifest["inputs"]]
        code, failure = ISO.run(args.native.resolve(), native_rows, attempt / "native.log", capture, 1 << 30)
        if code or failure:
            manifest["failure"] = failure or f"native process failed {code}"
    rows = score(capture, attempt, args.rust, args.native, manifest["go_exit"], manifest["failure"])
    report = {"tier": "png", "source_commit": references.SOURCE_COMMIT,
        "capture": str(capture), "go_exit": manifest["go_exit"], "failure": manifest["failure"],
        "rust": str(args.rust.resolve()), "rust_sha256": references.digest(args.rust) if args.rust.is_file() else None,
        "native": str(args.native.resolve()) if args.native else None,
        "native_sha256": references.digest(args.native) if args.native else None,
        "budgets": {"total": 1 << 30, "file": 128 << 20, "free": 20 << 30},
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "native_summary": dict(collections.Counter(r.get("native_outcome", "unscored") for r in rows)), "rows": rows}
    args.output.write_text(json.dumps(report, indent=1) + "\n")
    print(report["summary"], report["native_summary"])
    return int(bool(manifest["go_exit"] or manifest["failure"]))
