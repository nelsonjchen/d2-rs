#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Replay retained renderer witnesses with frozen binaries and fresh captures."""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=("multiline", "jupyter"), required=True)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--build-manifest", type=Path)
    args = parser.parse_args()
    binary = (ROOT / args.rust).resolve()
    destination = (ROOT / args.output).resolve()
    if not binary.is_file() or not binary.is_relative_to(ROOT / ".reference"):
        parser.error("--rust must name a frozen reference executable")
    if destination.exists() or not destination.is_relative_to(ROOT / ".reference"):
        parser.error("--output must name a fresh reference directory")
    probe_path = Path(__file__).with_name(
        "multiline-text-anchor-20261002.py"
        if args.suite == "multiline"
        else "tala-jupyter-render-20261002.py"
    )
    spec = importlib.util.spec_from_file_location("retained_renderer_probe", probe_path)
    probe = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(probe)
    removed = sorted(
        name for name in os.environ
        if name.startswith(("D2_TRACE_", "D2RS_", "D2_RS_", "TALA_TRACE_", "WEFTAN_"))
    )
    for name in removed:
        del os.environ[name]
    os.environ["GOMAXPROCS"] = "1"
    os.environ["RAYON_NUM_THREADS"] = "1"
    if args.suite == "multiline":
        sys.argv = [str(probe_path), "--rust", str(binary.relative_to(ROOT)),
                    "--output", str(destination.relative_to(ROOT)), "--extended"]
    else:
        probe.OUTPUT = destination
        probe.BINARIES["rust"] = binary
    exit_code = probe.main()
    report_path = destination / "report.json"
    report = json.loads(report_path.read_text())
    driver = destination / "replay-driver-source-at-run.py"
    driver.write_bytes(Path(__file__).read_bytes())
    report["replay_driver"] = probe.inventory(driver)
    report["replay_driver_environment_names_removed"] = removed
    report["replay_driver_environment_overrides"] = {
        "GOMAXPROCS": "1", "RAYON_NUM_THREADS": "1",
    }
    if args.build_manifest is not None:
        source = (ROOT / args.build_manifest).resolve()
        frozen = destination / "build-manifest-at-run.json"
        frozen.write_bytes(source.read_bytes())
        report["build_manifest_at_run"] = probe.inventory(frozen)
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    if args.suite == "multiline":
        return 0 if all(case["matched"] for case in report["cases"]) else 1
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
