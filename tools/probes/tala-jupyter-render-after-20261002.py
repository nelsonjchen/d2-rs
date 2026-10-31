#!/usr/bin/env python3
"""Replay the retained Jupyter renderer witness after the chunk-anchor fix."""

from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path


PROBE_PATH = Path(__file__).with_name("tala-jupyter-render-20261002.py")
SPEC = importlib.util.spec_from_file_location("jupyter_renderer_probe", PROBE_PATH)
PROBE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(PROBE)

PROBE.OUTPUT = PROBE.ROOT / ".reference/tala-jupyter-render-after-20261002"
PROBE.BINARIES["rust"] = (
    PROBE.ROOT / ".reference/bin/d2-rs-multiline-text-diagnostic-20261002"
)


def main() -> int:
    # The original run had no D2_RS_* parent settings; clear these explicitly
    # for this replay as well, without recording unrelated environment values.
    removed = sorted(key for key in os.environ if key.startswith("D2_RS_"))
    for key in removed:
        del os.environ[key]
    exit_code = PROBE.main()
    driver_snapshot = PROBE.OUTPUT / "replay-driver-source-at-run.py"
    driver_snapshot.write_bytes(Path(__file__).read_bytes())
    report_path = PROBE.OUTPUT / "report.json"
    report = json.loads(report_path.read_text())
    report["replay_driver"] = PROBE.inventory(driver_snapshot)
    report["replay_driver_environment_names_removed"] = removed
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
