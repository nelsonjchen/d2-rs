#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Replay the authored ordinary-dash witness with frozen native executables."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
OFFICIAL_SHA = "e8ba501a413bac33a4d93014c475b804de10fb2863a391583e23a8d88160e311"


def inventory(path: Path) -> dict:
    data = path.read_bytes()
    return {"path": str(path.relative_to(ROOT)), "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest()}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", required=True, type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--build-manifest", required=True, type=Path)
    args = parser.parse_args()
    output = (ROOT / args.output).resolve()
    if output.exists() or not output.is_relative_to(ROOT / ".reference"):
        parser.error("--output must name a fresh reference directory")
    binaries = {"go": ROOT / ".reference/bin/d2-v0.9.0-official",
                "rust": (ROOT / args.rust).resolve()}
    if args.baseline:
        binaries["baseline"] = (ROOT / args.baseline).resolve()
    for path in binaries.values():
        if not path.is_file() or not path.is_relative_to(ROOT / ".reference"):
            parser.error("executables must be frozen reference files")
    binary_records = {name: inventory(path) for name, path in binaries.items()}
    assert binary_records["go"]["sha256"] == OFFICIAL_SHA
    output.mkdir()
    source = ROOT / "tests/d2/tala-ordinary-dash-precision-rect.d2"
    (output / "source.d2").write_bytes(source.read_bytes())
    (output / "probe-source-at-run.py").write_bytes(Path(__file__).read_bytes())
    manifest = (ROOT / args.build_manifest).resolve()
    (output / "build-manifest-at-run.json").write_bytes(manifest.read_bytes())
    environment = dict(os.environ)
    removed = sorted(name for name in environment if name.startswith(
        ("D2_TRACE_", "D2RS_", "D2_RS_", "TALA_TRACE_")))
    for name in removed:
        del environment[name]
    environment.update(GOMAXPROCS="1", RAYON_NUM_THREADS="1")
    report = {"schema_version": 1,
              "scope": "Authored ordinary-dashed rectangle, decoded PNG, same-side public SVG and diagnostic noninterference; not full renderer qualification.",
              "source": inventory(source), "binaries": binary_records,
              "build_manifest": inventory(output / "build-manifest-at-run.json"),
              "environment_removed_names": removed,
              "environment_overrides": {"GOMAXPROCS": "1", "RAYON_NUM_THREADS": "1"},
              "runs": []}

    def run(name: str, extension: str, observer: bool = False) -> None:
        label = name + ("-observer" if observer else "")
        target = output / f"{label}.{extension}"
        argv = [str(binaries[name]), "--layout=" + ("tala" if name == "go" else "tala-rs"),
                "--tala-seeds=1"]
        if name == "go":
            argv += [str(source), str(target)]
        else:
            argv += [f"--stdout-format={extension}", str(source), "-"]
        extra = {"D2_RS_DASH_RUN_TRACE": str(output / "native-dash-runs.txt")} if observer else {}
        start = time.monotonic()
        result = subprocess.run(argv, cwd=ROOT, env=environment | extra, capture_output=True,
                                timeout=120)
        for kind, data in (("stdout", result.stdout), ("stderr", result.stderr)):
            (output / f"{label}.{extension}.{kind}").write_bytes(data)
        if name != "go":
            target.write_bytes(result.stdout)
        report["runs"].append({"name": label, "format": extension, "argv": argv,
                                "environment_overrides": extra, "exit_code": result.returncode,
                                "seconds": time.monotonic() - start})
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        assert result.returncode == 0, f"{label} {extension} failed"

    for name in binaries:
        for extension in ("svg", "png"):
            run(name, extension)
    run("rust", "png", observer=True)
    helper = ROOT / "scripts/check-render-parity.py"
    (output / "pixel-decoder-source-at-run.py").write_bytes(helper.read_bytes())
    spec = importlib.util.spec_from_file_location("pixel_decoder", helper)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    decoded = {name: module.decode_png(output / f"{name}.png") for name in binaries}
    width, height, expected = decoded["go"]
    comparisons = []
    for name in binaries:
        if name == "go":
            continue
        actual_width, actual_height, actual = decoded[name]
        same_size = (width, height) == (actual_width, actual_height)
        comparisons.append({"name": name, "go_size": [width, height],
                            "native_size": [actual_width, actual_height],
                            "matched": same_size and expected == actual,
                            "different_pixels": sum(expected[i:i + 3] != actual[i:i + 3]
                                                    for i in range(0, len(expected), 3)) if same_size else None,
                            "decoded_rgb_sha256": hashlib.sha256(actual).hexdigest()})
    report["pixel_comparisons"] = comparisons
    report["official_decoded_rgb_sha256"] = hashlib.sha256(expected).hexdigest()
    report["observer_png_equal"] = (output / "rust.png").read_bytes() == (output / "rust-observer.png").read_bytes()
    report["observer_trace_created"] = (output / "native-dash-runs.txt").is_file()
    report["public_svg_preserved"] = None
    if "baseline" in binaries:
        report["public_svg_preserved"] = (output / "rust.svg").read_bytes() == (output / "baseline.svg").read_bytes()
    report["public_svg_has_no_private_dash_metadata"] = b"data-d2-rs-dash" not in (output / "rust.svg").read_bytes()
    report["binaries_unchanged"] = binary_records == {name: inventory(path) for name, path in binaries.items()}
    report["artifacts"] = [inventory(path) for path in sorted(output.rglob("*"))
                           if path.is_file() and path.name != "report.json"]
    report["passed"] = next(item["matched"] for item in comparisons if item["name"] == "rust") and report["observer_png_equal"] and report["binaries_unchanged"] and report["public_svg_has_no_private_dash_metadata"] and report["public_svg_preserved"] is not False
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("passed", "pixel_comparisons", "public_svg_preserved", "observer_png_equal", "observer_trace_created")}), flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
