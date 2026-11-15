#!/usr/bin/env python3
"""Compare d2-rs and the pinned Go D2 oracle on every e2e case and format.

perf-bench.py times 12 SVG cases; this sweep covers the whole corpus: every
case with every engine as SVG, plus PNG, PDF, PPTX and GIF through Dagre.
Pass 1 times each job once per implementation; pass 2 re-times every job
over --flag (wall or peak RSS ratio) with --repeat interleaved runs and
reports medians. Each implementation writes the same output file name in
its own directory, because PPTX embeds that name. Outputs are hashed so a
speed-up cannot hide a parity change (only PDF CreationDate/ModDate are
normalized). Pass --isometric to measure isometric exports. Writes a JSON
report and prints a per-format summary and every row still slower or larger
than Go.

usage: corpus-perf-sweep.py --rust <d2-rs> --output <report.json>
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import re
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import references

ENGINES = ("dagre", "elk", "tala")
EXPORTS = (("png", ()), ("pdf", ()), ("pptx", ()), ("gif", ("--animate-interval=125",)))
REMOTE_ICON = re.compile(r"^[^#\n]*\bicon\s*:\s*\"?https?://", re.M)

_spec = importlib.util.spec_from_file_location("perf_bench", Path(__file__).with_name("perf-bench.py"))
perf_bench = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(perf_bench)


def corpus_jobs(isometric=False):
    jobs = []
    for path in sorted(references.E2E_CASES.glob("*.json")):
        case = json.loads(path.read_text())
        script = case.get("script") or ""
        # Remote icons time the network, not d2 (as in perf-bench).
        if script.strip() and not REMOTE_ICON.search(script):
            jobs += [(case["name"], script, engine, "svg", ()) for engine in ENGINES]
            jobs += [(case["name"], script, "dagre", fmt, extra) for fmt, extra in EXPORTS]
    if isometric:
        jobs = [(name, script, engine, fmt, (*extra, "--isometric"))
                for name, script, engine, fmt, extra in jobs]
    return jobs


def run_job(job, implementations, work, repeat):
    name, script, engine, fmt, extra = job
    samples = {impl: [] for impl, _ in implementations}
    for _ in range(repeat):
        for impl, binary in implementations:
            directory = work / impl
            directory.mkdir(exist_ok=True)
            source = directory / "input.d2"
            source.write_text(script)
            output = directory / f"out.{fmt}"
            exit_code, wall, cpu, rss = perf_bench.run(binary, engine, source, output, list(extra))
            samples[impl].append((exit_code, wall, rss, output_digest(output), cpu))
    median = {impl: {"exit": s[-1][0], "wall": statistics.median(x[1] for x in s),
                     "rss": statistics.median(x[2] for x in s), "hash": s[-1][3],
                     "cpu": statistics.median(x[4] for x in s),
                     "sample_exits": [x[0] for x in s], "output_hashes": [x[3] for x in s]}
              for impl, s in samples.items()}
    go, rust = median["go"], median["rust"]
    consistent = all(median[impl]["sample_exits"] == [go["exit"]] * repeat
                     and all(x == go["hash"] for x in median[impl]["output_hashes"])
                     for impl in ("go", "rust"))
    identical = go["exit"] == 0 and go["hash"] is not None and consistent
    error_match = False
    if go["exit"] and rust["exit"] and go["hash"] == rust["hash"]:
        # Both refused the input (e.g. Go's GIF pixel limit). That is parity
        # when the error text and any partial outputs match in every repeat;
        # the timed runs discard stderr.
        errors = {impl: failure_text(binary, engine, work / impl, fmt, extra)
                  for impl, binary in implementations}
        error_match = identical = consistent and errors["go"] == errors["rust"]
    return {"case": name, "engine": engine, "format": fmt, "extra": list(extra), "go": go, "rust": rust, "repeat": repeat,
            "wall_ratio": rust["wall"] / max(go["wall"], 1e-3), "rss_ratio": rust["rss"] / max(go["rss"], 1),
            "identical": identical, "error_match": error_match}


def output_digest(output):
    """Hash a single output file, or a multi-board output directory."""
    if output.is_file():
        if not 0 < output.stat().st_size <= 128 << 20:
            raise ValueError(f"empty output or capture budget exceeded: {output}")
        return hashlib.sha256(perf_bench.output_bytes(output)).hexdigest()
    board_output = output.with_suffix("")
    if board_output.is_dir():
        inventory = perf_bench.output_inventory(board_output)
        return hashlib.sha256(json.dumps(inventory, sort_keys=True).encode()).hexdigest() if inventory else None
    return None


def failure_text(binary, engine, directory, fmt, extra):
    """Rerun a failed job once and return its stderr with the job directory removed."""
    source, output = directory / "input.d2", directory / f"out.{fmt}"
    result = subprocess.run([str(binary.resolve()), f"--layout={engine}", *extra, str(source), str(output)],
                            capture_output=True, text=True, timeout=610, check=False)
    return result.stderr.replace(str(directory), "<job>").strip()


def summarize(rows):
    lines = ["format engine  rows  median-wall  slower  median-rss  larger"]
    for key in sorted({(r["format"], r["engine"]) for r in rows}):
        group = [r for r in rows if (r["format"], r["engine"]) == key and r["go"]["exit"] == 0]
        if not group:
            continue
        lines.append(f"{key[0]:6} {key[1]:6} {len(group):5} {statistics.median(r['wall_ratio'] for r in group):11.2f}"
                     f" {sum(r['wall_ratio'] > 1 for r in group):7}"
                     f" {statistics.median(r['rss_ratio'] for r in group):11.2f} {sum(r['rss_ratio'] > 1 for r in group):7}")
    return lines


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--go", type=Path, default=references.GO_ORACLE)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--flag", type=float, default=1.0, help="re-time rows whose wall or RSS ratio exceeds this")
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--isometric", action="store_true")
    parser.add_argument("--formats", default="svg,png,pdf,pptx,gif", help="comma-separated formats")
    parser.add_argument("--engines", default=",".join(ENGINES), help="comma-separated engines")
    parser.add_argument("--case", default="", help="regular expression selecting case names")
    args = parser.parse_args()
    implementations = (("go", args.go.resolve()), ("rust", args.rust.resolve()))
    formats, engines = args.formats.split(","), args.engines.split(",")
    if set(formats) - {"svg", *(fmt for fmt, _ in EXPORTS)} or set(engines) - set(ENGINES):
        parser.error("unknown format or engine")
    jobs = [job for job in corpus_jobs(args.isometric)
            if job[2] in engines and job[3] in formats and re.search(args.case, job[0])]
    if not jobs:
        parser.error("no jobs selected")
    start = time.time()
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        rows = []
        for index, job in enumerate(jobs):
            rows.append(run_job(job, implementations, work, 1))
            if index % 20 == 0:
                print(f"pass 1: {index}/{len(jobs)} after {time.time() - start:.0f} s", flush=True)
        flagged = [k for k, row in enumerate(rows)
                   if row["go"]["exit"] == 0 and max(row["wall_ratio"], row["rss_ratio"]) > args.flag]
        print(f"pass 2: re-timing {len(flagged)} rows", flush=True)
        for k in flagged:
            rows[k] = run_job(jobs[k], implementations, work, args.repeat)
    over = [r for r in rows if r["go"]["exit"] == 0 and max(r["wall_ratio"], r["rss_ratio"]) > 1.0]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps({"source_commit": references.SOURCE_COMMIT, "isometric": args.isometric,
                                      "rows": rows, "over_go": over}, indent=1) + "\n")
    print("\n".join(summarize(rows)))
    print(f"{len(over)} rows slower or larger than Go ({time.time() - start:.0f} s):")
    for r in sorted(over, key=lambda r: -max(r["wall_ratio"], r["rss_ratio"])):
        print(f"  wall {r['wall_ratio']:.2f}x rss {r['rss_ratio']:.2f}x  {r['format']} {r['engine']} {r['case']}"
              f"  (Go {r['go']['wall']:.3f} s, d2-rs {r['rust']['wall']:.3f} s)")


if __name__ == "__main__":
    main()
