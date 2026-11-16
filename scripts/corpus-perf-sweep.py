#!/usr/bin/env python3
"""Compare d2-rs and the pinned Go D2 oracle on every e2e case and format.

This sweep covers every e2e case with every engine as SVG, PNG, PDF, PPTX,
GIF and TXT, supplementing perf-bench.py's fixed performance gate.
Pass 1 times each job once per implementation; pass 2 re-times every job
over --flag (wall or peak RSS ratio) with --repeat interleaved runs and
reports medians. Each implementation writes the same output file name in
its own directory, because PPTX embeds that name. Outputs are hashed so a
speed-up cannot hide a parity change, with perf-bench.py's compare_samples
rules: non-PDF Go repeats must agree and equal every Rust repeat, and a
non-PDF Go disagreement is resolved only by the seedable-map oracle
(--go-mapseed, --mapseed-seeds; scripts/mapseed.py): mapseed-match or
known-mapseed when every Rust repeat equals a seeded Go output, else
go-nondeterministic-no-rust-match or mapseed-oracle-missing. Rust PDFs must
equal an observed Go PDF with only the CreationDate/ModDate digits masked. Pass --isometric to measure isometric
exports. Writes a JSON report and prints a per-format summary, outcome counts
and every row still slower or larger than Go.

usage: corpus-perf-sweep.py --rust <d2-rs> --output <report.json>
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import mapseed
import references

ENGINES = ("dagre", "elk", "tala")
EXPORTS = (("png", ()), ("pdf", ()), ("pptx", ()), ("gif", ("--animate-interval=125",)), ("txt", ()))
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
            jobs += [(case["name"], script, engine, fmt, extra)
                     for engine in ENGINES for fmt, extra in (("svg", ()), *EXPORTS)]
    if isometric:
        jobs = [(name, script, engine, fmt, (*extra, "--isometric"))
                for name, script, engine, fmt, extra in jobs]
    return jobs


def seeded_digest(oracle, engine, source, directory, fmt, extra, seed):
    """One untimed seeded-oracle run hashed like output_digest, or None on failure."""
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    output = directory / f"out.{fmt}"
    result = subprocess.run([str(Path(oracle).resolve()), f"--layout={engine}", *extra, str(source), str(output)],
                            capture_output=True, timeout=610, check=False, env={**os.environ, mapseed.ENV: str(seed)})
    try:
        return output_digest(output) if result.returncode == 0 else None
    except ValueError:
        return None


def run_job(job, implementations, work, repeat, oracle=None, seeds=mapseed.DEFAULT_SEEDS, registry=None):
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
    known = bool(registry and registry.known(mapseed.identity(script, [f"--layout={engine}", *extra, fmt])))
    comparison = perf_bench.compare_samples(
        fmt, go, rust,
        lambda seed: seeded_digest(oracle.path, engine, work / "go" / "input.d2", work / "go-mapseed", fmt, extra, seed),
        oracle, seeds, known)
    identical = comparison["outcome"] in mapseed.PASSING
    error_match = False
    if go["exit"] and rust["exit"] and go["hash"] == rust["hash"]:
        # Both refused the input (e.g. Go's GIF pixel limit). That is parity
        # when the error text and any partial outputs match in every repeat;
        # the timed runs discard stderr.
        errors = {impl: failure_text(binary, engine, work / impl, fmt, extra)
                  for impl, binary in implementations}
        error_match = identical = consistent and errors["go"] == errors["rust"]
        if error_match:
            comparison["outcome"] = "error-match"
    return {"case": name, "engine": engine, "format": fmt, "extra": list(extra), "go": go, "rust": rust, "repeat": repeat,
            "wall_ratio": rust["wall"] / max(go["wall"], 1e-3), "rss_ratio": rust["rss"] / max(go["rss"], 1),
            "identical": identical, "error_match": error_match, **comparison}


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
    parser.add_argument("--formats", default="svg,png,pdf,pptx,gif,txt", help="comma-separated formats")
    parser.add_argument("--engines", default=",".join(ENGINES), help="comma-separated engines")
    parser.add_argument("--case", default="", help="regular expression selecting case names")
    mapseed.add_arguments(parser)
    args = parser.parse_args()
    oracle = perf_bench.mapseed_oracle(args.go, args.go_mapseed)
    registry = mapseed.Registry(args.mapseed_registry)
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
            rows.append(run_job(job, implementations, work, 1, oracle, args.mapseed_seeds, registry))
            if index % 20 == 0:
                print(f"pass 1: {index}/{len(jobs)} after {time.time() - start:.0f} s", flush=True)
        flagged = [k for k, row in enumerate(rows)
                   if row["go"]["exit"] == 0 and max(row["wall_ratio"], row["rss_ratio"]) > args.flag]
        print(f"pass 2: re-timing {len(flagged)} rows", flush=True)
        for k in flagged:
            rows[k] = run_job(jobs[k], implementations, work, args.repeat, oracle, args.mapseed_seeds, registry)
    over = [r for r in rows if r["go"]["exit"] == 0 and max(r["wall_ratio"], r["rss_ratio"]) > 1.0]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps({"source_commit": references.SOURCE_COMMIT, "isometric": args.isometric,
                                      "rows": rows, "over_go": over}, indent=1) + "\n")
    print("\n".join(summarize(rows)))
    outcomes = {outcome: sum(r["outcome"] == outcome for r in rows) for outcome in sorted({r["outcome"] for r in rows})}
    pdf_variants = max((r["go_variants"] for r in rows if r["format"] == "pdf"), default=0)
    print(f"outcomes {outcomes}; max PDF Go variants {pdf_variants}")
    for r in rows:
        if r["outcome"] in (mapseed.NO_RUST_MATCH, mapseed.ORACLE_MISSING):
            print(f"FAIL {r['outcome']}: {r['format']} {r['engine']} {r['case']}")
        elif r["outcome"] in (mapseed.MAPSEED_MATCH, mapseed.KNOWN_MAPSEED):
            print(f"{r['outcome']} (seeds {','.join(map(str, r['mapseed']['mapseed_seeds']))}): "
                  f"{r['format']} {r['engine']} {r['case']}")
    if any(r["outcome"] == mapseed.ORACLE_MISSING for r in rows):
        print(f"error: {oracle.message()}", file=sys.stderr)
    if args.record_mapseed:
        jobs_by_row = {(job[0], job[2], job[3], tuple(job[4])): job for job in jobs}
        added = mapseed.record_cases(registry, (
            (mapseed.identity(job[1], [f"--layout={job[2]}", *job[4], job[3]]), job[1], [f"--layout={job[2]}", *job[4], job[3]],
             f"corpus-perf-sweep.py {r['case']} {r['engine']} {r['format']}",
             mapseed.Resolution(r["outcome"], r["mapseed"]["mapseed_seeds"], r["mapseed"]["go_variants"]))
            for r in rows if r.get("mapseed", {}).get("go_variants", 1) > 1
            for job in [jobs_by_row[(r["case"], r["engine"], r["format"], tuple(r["extra"]))]]), "corpus-perf-sweep")
        print(f"recorded {added} new map-order-dependent inputs in {registry.path}")
    print(f"{len(over)} rows slower or larger than Go ({time.time() - start:.0f} s):")
    for r in sorted(over, key=lambda r: -max(r["wall_ratio"], r["rss_ratio"])):
        print(f"  wall {r['wall_ratio']:.2f}x rss {r['rss_ratio']:.2f}x  {r['format']} {r['engine']} {r['case']}"
              f"  (Go {r['go']['wall']:.3f} s, d2-rs {r['rust']['wall']:.3f} s)")


if __name__ == "__main__":
    main()
