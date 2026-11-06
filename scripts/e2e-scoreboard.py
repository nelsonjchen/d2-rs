#!/usr/bin/env python3
"""Score d2-rs against every upstream D2 e2e golden board at the pin.

Cases are dumped from the pinned e2etests package by the env-gated hook in the
trace checkout's e2etests/e2e_test.go (D2RS_E2E_DUMP=<dir>; the patch is
tools/patches/d2-e2e-case-dump.patch), then each case is
rendered by the Rust CLI with the e2e render options (pad 0, unbundled, TALA
seed 1) for every engine upstream tests. Two tiers are scored per board:

* target: the `d2-<hash>` SVG class, which is D2's hash of the target diagram
  (shapes, connections, root and config). Equal hashes mean equal target JSON.
* svg: SVG bytes after the only normalizations the goldens need: the XML tag,
  the version string the goldens were generated with, and the trailing newline.

Missing output, crashes and timeouts are failures, never skips. Cases with
injected measured texts cannot be driven through the CLI, and the CLI writes no
file for an empty diagram; both are reported as inapplicable, never passing.
"""

from __future__ import annotations

import argparse
import references
import collections
import concurrent.futures
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
UPSTREAM = ROOT.parent / "d2"
HASH = re.compile(r'class="d2-([0-9]+) d2-svg"')
# d2animate.Wrap names every frame's keyframes after the master hash (the
# recursive HashID of the whole board tree); per-board classes are SVG-tier.
ANIMATED_HASH = re.compile(r'd2Transition-d2-([0-9]+)-0\b')


def target_hash(svg: str, animated: bool) -> str | None:
    match = ANIMATED_HASH.search(svg) if animated else HASH.search(svg)
    return match.group(1) if match else None


def is_animated(svg: str) -> bool:
    return svg.lstrip().startswith("<svg") and ' d2Version="' in svg.split(">", 1)[0]


def normalize(svg: str) -> str:
    svg = svg.replace('<?xml version="1.0" encoding="utf-8"?>', "", 1)
    svg = re.sub(r'data-d2-version="[^"]*"', 'data-d2-version=""', svg)
    svg = re.sub(r'd2Version="[^"]*"', 'd2Version=""', svg, count=1)
    return svg.rstrip("\n")


RUST_ENV: dict[str, str] = {}


def run_case(binary: Path, case: dict, engine: str, timeout: int, go: Path | None = None) -> dict:
    row = run_binary(binary, case, engine, timeout, RUST_ENV)
    if go is not None and not row["outcome"].startswith(("inapplicable", "no-golden")):
        go_row = run_binary(go, case, engine, timeout)
        row["go_outcome"] = go_row["outcome"]
        if "hash" in row and "hash" in go_row:
            row["rust_go_target_match"] = row["hash"] == go_row["hash"]
    return row


def run_binary(binary: Path, case: dict, engine: str, timeout: int, env: dict[str, str] | None = None) -> dict:
    name = case["name"]
    expected_svg = UPSTREAM / "e2etests/testdata" / name / engine / "sketch.exp.svg"
    row: dict = {"name": name, "engine": engine}
    if case["hasMtexts"]:
        row["outcome"] = "inapplicable-measured-texts"
        return row
    expected_error = case["expErr"] or (case["talaLayoutError"] if engine == "tala" else "")
    feature_error = {"dagre": case["dagreFeatureError"], "elk": case["elkFeatureError"]}.get(engine, "")
    expected_error = expected_error or feature_error
    if not expected_error and not expected_svg.is_file():
        row["outcome"] = "no-golden"
        return row
    expected = normalize(expected_svg.read_text()) if expected_svg.is_file() else ""
    with tempfile.TemporaryDirectory(prefix="d2rs-e2e-") as scratch:
        source = Path(scratch) / "input.d2"
        output = Path(scratch) / "output.svg"
        source.write_text(case["script"])
        command = [str(binary), "--pad=0", "--bundle=false", "--no-xml-tag", "--omit-version=false",
                   "--tala-seeds=1", f"--layout={engine}", "--timeout=" + str(timeout)]
        if case["themeID"] is not None:
            command.append(f"--theme={case['themeID']}")
        if is_animated(expected):
            command.append("--animate-interval=1000")
        command += [str(source), str(output)]
        try:
            process = subprocess.run(command, capture_output=True, timeout=timeout + 15, check=False,
                                     env={**os.environ, **(env or {})})
        except subprocess.TimeoutExpired:
            row["outcome"] = "timeout"
            return row
        stderr = process.stderr.decode(errors="replace")
        if expected_error:
            row["expected_error"] = expected_error
            if process.returncode != 0 and expected_error in stderr:
                row["outcome"] = "error-match"
            else:
                row["outcome"] = "error-mismatch"
                row["stderr"] = stderr[-1500:]
            return row
        if process.returncode == 0 and not output.exists() and not case["script"].strip():
            # The CLI writes nothing for an empty diagram; e2e renders a board.
            row["outcome"] = "inapplicable-empty-cli-output"
            return row
        if process.returncode != 0 or not output.is_file() or output.stat().st_size == 0:
            row["outcome"] = "rust-failed"
            row["exit"] = process.returncode
            row["stderr"] = stderr[-1500:]
            return row
        actual = normalize(output.read_text(errors="replace"))
    animated = is_animated(expected)
    expected_hash = target_hash(expected, animated)
    actual_hash = target_hash(actual, animated)
    if actual_hash:
        row["hash"] = actual_hash
    row["target_match"] = bool(expected_hash and actual_hash and expected_hash == actual_hash)
    row["svg_match"] = actual == expected
    row["outcome"] = "svg-match" if row["svg_match"] else ("target-match" if row["target_match"] else "target-mismatch")
    return row


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cases", type=Path, default=references.E2E_CASES)
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    parser.add_argument("--output", type=Path, default=references.REFERENCE / "scoreboard-master-6a7b4837/e2e-report.json")
    parser.add_argument("--go", type=Path, default=None, help="also score this Go CLI as the reference")
    parser.add_argument("--d2svg", action="store_true", help="render SVG through the d2svg port (D2_RS_SVG_RENDERER=d2svg)")
    parser.add_argument("--engines", default="dagre,elk,tala")
    parser.add_argument("--filter", default="")
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument("--timeout", type=int, default=60)
    args = parser.parse_args()
    if args.d2svg:
        RUST_ENV["D2_RS_SVG_RENDERER"] = "d2svg"
    cases = [json.loads(path.read_text()) for path in sorted(args.cases.glob("*.json"))]
    if not cases:
        parser.error(f"no dumped cases in {args.cases}")
    engines = args.engines.split(",")
    jobs = []
    for case in cases:
        if args.filter and not re.search(args.filter, case["name"]):
            continue
        for engine in ["dagre"] if case["justDagre"] else ["dagre", "elk", "tala"]:
            if engine in engines:
                jobs.append((case, engine))
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        rows = list(pool.map(lambda job: run_case(args.rust, job[0], job[1], args.timeout, args.go), jobs))
    summary: dict = collections.defaultdict(collections.Counter)
    for row in rows:
        summary[row["engine"]][row["outcome"]] += 1
        summary["all"][row["outcome"]] += 1
        if "go_outcome" in row:
            summary["go-" + row["engine"]][row["go_outcome"]] += 1
    report = {"rust": str(args.rust), "cases": len(cases), "boards": len(rows),
              "summary": {k: dict(v) for k, v in summary.items()}, "rows": rows}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=1) + "\n")
    for engine, counts in report["summary"].items():
        print(engine, dict(sorted(counts.items())))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
