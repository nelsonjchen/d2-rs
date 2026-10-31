#!/usr/bin/env python3
"""Scoreboard-capture generator for crates/d2-rs-render oracle fixtures
(tools/fixtures/renderexport-render-api.toml).

    --script scripts/<scoreboard>.py --output <logical> [--suite NAME] [--package PKG]
            [--select <seed json>] [--format indent2|compact]

Runs the scoreboard's original-test capture (pinned upstream test bodies with
call replacements plus its hook) against $D2RS_UPSTREAM through a go overlay
(scripts/upstream_overlay.py; the shared tree is never written), checks it with
the scoreboard's own verify(), and writes the verified case list. `--suite`
selects a svg-admission suite (structural/labels); `--package` keeps the cases
of one Go package (renderer-admission); `--digest-output` reduces rendered
SVGs to their sha256 (the svg-admission readers compare digests); `--select` keeps the listed case names,
in the listed order. Cargo tests do the comparison. Run through
scripts/regen-fixtures.py (docs/FIXTURES.md).
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import fixture_recipe  # noqa: E402
import upstream_overlay  # noqa: E402


def load(script: Path):
    spec = importlib.util.spec_from_file_location(script.stem.replace("-", "_"), script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    # svg-font-scoreboard patches the svg-admission module it calls `base`;
    # themes-api-scoreboard defines its own capture and only borrows helpers.
    return module if hasattr(module, "capture_original") else module.base


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--script", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--suite")
    parser.add_argument("--package")
    parser.add_argument("--select")
    parser.add_argument("--drop", action="append", default=[], help="case field to leave out")
    parser.add_argument("--digest-output", action="store_true",
                        help="svg-admission layout: {name, request, error, output_sha256 of the base64 output}")
    parser.add_argument("--format", choices=["indent2", "compact"], default="indent2")
    args = parser.parse_args()
    work = fixture_recipe.workdir()
    upstream_overlay.redirect(work / "overlay")
    score = load(ROOT / args.script)
    if args.suite:
        getattr(score, args.suite + "_suite")()
    capture = work / "capture"
    score.capture_original(capture)
    cases = score.verify(capture)
    if args.package:
        cases = [c for c in cases if c["package"] == args.package]
    if args.select:
        names = json.loads((ROOT / args.select).read_text())
        by_name = {c["name"]: c for c in cases}
        missing = [n for n in names if n not in by_name]
        assert not missing, f"selection names cases the capture lacks: {missing}"
        cases = [by_name[n] for n in names]
    if args.digest_output:
        cases = [{"name": c["name"], "request": c["request"], "error": c["got"]["error"],
                  "output_sha256": None if c["got"]["output"] is None
                  else hashlib.sha256(base64.b64decode(c["got"]["output"])).hexdigest()} for c in cases]
    cases = [{k: v for k, v in c.items() if k not in args.drop} for c in cases]
    assert cases, "empty capture"
    kwargs = {"indent2": dict(indent=2), "compact": dict(separators=(",", ":"))}[args.format]
    fixture_recipe.output(args.output).write_text(json.dumps(cases, **kwargs) + "\n")


if __name__ == "__main__":
    main()
