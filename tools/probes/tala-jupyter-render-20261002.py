#!/usr/bin/env python3
"""Retain current Jupyter SVG/PNG witnesses against the official release.

This is a bounded investigation, not full renderer or TALA qualification.
Raw output is retained under .reference; previous failures are not overwritten.
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import time


ROOT = Path(__file__).resolve().parents[2]
OUTPUT = ROOT / ".reference/tala-jupyter-render-investigation-20261002"
SOURCE = ROOT / "../d2/e2etests/testdata/files/jupyter_k8s_oidc.d2"  # pristine pin (was tests/upstream/d2)
BINARIES = {
    "go": ROOT / ".reference/bin/d2-v0.9.0-official",
    "rust": ROOT / ".reference/bin/d2-rs-cluster-alignment-diagnostic-20261002",
}


def inventory(path: Path) -> dict[str, object]:
    data = path.read_bytes()
    return {
        "path": str(path.relative_to(ROOT)),
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
    }


def main() -> int:
    OUTPUT.mkdir(parents=True, exist_ok=False)
    environment = dict(os.environ)
    removed = sorted(
        key for key in environment
        if key.startswith(("D2_TRACE_", "D2RS_", "TALA_TRACE_", "WEFTAN_"))
    )
    for key in removed:
        del environment[key]
    environment["GOMAXPROCS"] = "1"
    report: dict[str, object] = {
        "schema_version": 1,
        "scope": "Jupyter current default options; native versus official SVG/decoded PNG witnesses, not byte-exact SVG or full D2 parity.",
        "reference_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "source": inventory(SOURCE),
        "binaries": {name: inventory(path) for name, path in BINARIES.items()},
        "probe_source": inventory(Path(__file__)),
        "removed_environment_names": removed,
        "environment_overrides": {"GOMAXPROCS": "1"},
        "runs": [],
    }
    (OUTPUT / "source.d2").write_bytes(SOURCE.read_bytes())
    (OUTPUT / "probe-source-at-run.py").write_bytes(Path(__file__).read_bytes())
    for name, binary in BINARIES.items():
        for extension in ("svg", "png"):
            target = OUTPUT / f"{name}.{extension}"
            command = [str(binary), f"--layout={'tala' if name == 'go' else 'tala-rs'}"]
            if name == "rust":
                command.append(f"--stdout-format={extension}")
                command.extend([str(SOURCE), "-"])
            else:
                command.extend([str(SOURCE), str(target)])
            started = time.monotonic()
            result = subprocess.run(command, cwd=ROOT, env=environment, capture_output=True)
            (OUTPUT / f"{name}.{extension}.stdout").write_bytes(result.stdout)
            (OUTPUT / f"{name}.{extension}.stderr").write_bytes(result.stderr)
            if name == "rust":
                target.write_bytes(result.stdout)
            report["runs"].append({
                "argv": command, "cwd": str(ROOT), "exit_code": result.returncode,
                "elapsed_seconds": time.monotonic() - started,
                "stdout": inventory(OUTPUT / f"{name}.{extension}.stdout"),
                "stderr": inventory(OUTPUT / f"{name}.{extension}.stderr"),
                "output": inventory(target) if target.exists() else None,
            })
            (OUTPUT / "report.json").write_text(json.dumps(report, indent=2) + "\n")
            print(json.dumps({"engine": name, "format": extension, "exit_code": result.returncode}), flush=True)
            if result.returncode:
                return 1

    helper_path = ROOT / "scripts/check-render-parity.py"
    spec = importlib.util.spec_from_file_location("render_parity", helper_path)
    helper = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(helper)
    report["pixel_decoder_source"] = inventory(helper_path)
    width_go, height_go, pixels_go = helper.decode_png(OUTPUT / "go.png")
    width_rust, height_rust, pixels_rust = helper.decode_png(OUTPUT / "rust.png")
    same_size = (width_go, height_go) == (width_rust, height_rust)
    if same_size:
        difference = helper.summarize_pixel_differences(width_go, height_go, pixels_go, pixels_rust)
        deltas = [abs(expected - actual) for expected, actual in zip(pixels_go, pixels_rust)]
        report["pixels"] = {
            "matched": pixels_go == pixels_rust,
            "size": [width_go, height_go],
            "different_pixels": sum(any(deltas[i:i + 3]) for i in range(0, len(deltas), 3)),
            "mean_absolute_channel_error": sum(deltas) / len(deltas),
            "decoded_rgb_sha256": {
                "go": hashlib.sha256(pixels_go).hexdigest(),
                "rust": hashlib.sha256(pixels_rust).hexdigest(),
            },
            "diff_summary": {
                "bounds": difference["bounds"],
                "row_counts": [{k: v for k, v in row.items() if k != "delta_histogram"} for row in difference["rows"]],
            } if difference else None,
        }
        if difference:
            (OUTPUT / "pixel-difference.json").write_text(json.dumps(difference, indent=2) + "\n")
            report["raw_pixel_difference"] = inventory(OUTPUT / "pixel-difference.json")
    else:
        report["pixels"] = {"matched": False, "go_size": [width_go, height_go], "rust_size": [width_rust, height_rust]}
    (OUTPUT / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report["pixels"].items() if k != "diff_summary"}), flush=True)
    return 0 if report["pixels"]["matched"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
