#!/usr/bin/env python3
"""Benchmark verified official D2 against its archived external Weftan plugin.

This is an end-to-end baseline, not an engine-only or native d2-rs measurement.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile
import time

from references import ROOT, MANIFEST, release, host_name, verify_source, digest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plugin", type=Path, default=ROOT.parent / "weftan/target/release/d2plugin-weftan")
    parser.add_argument("--runs", type=int, default=10)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--threads", type=int, default=4)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("inputs", nargs="+", type=Path)
    args = parser.parse_args()
    if args.runs < 2 or args.warmups < 0 or args.threads < 1:
        parser.error("require runs >= 2, warmups >= 0, threads >= 1")
    verify_source()
    binary = release(host_name(), False)
    plugin = args.plugin.resolve(strict=True)
    if plugin.name != "d2plugin-weftan":
        parser.error("plugin must be named d2plugin-weftan for D2 discovery")
    environment = {k: v for k, v in os.environ.items() if not k.startswith(("D2_", "WEFTAN_")) and k not in ("DEV_MODE", "TEST_MODE", "DEBUG", "SCALE")}
    environment.update(PATH=str(plugin.parent) + os.pathsep + environment.get("PATH", ""),
                       GOMAXPROCS=str(args.threads), RAYON_NUM_THREADS=str(args.threads))
    report = {"schema_version": 1, "kind": "end_to_end_d2_cli",
              "d2_archive_sha256": MANIFEST["releases"][host_name()]["sha256"],
              "d2_binary_sha256": digest(binary), "plugin_sha256": digest(plugin),
              "d2_toolchain": MANIFEST["d2"]["go_toolchain"],
              "platform": platform.platform(), "logical_cpus": os.cpu_count(),
              "thread_limit": args.threads, "seed": args.seed, "runs": args.runs,
              "warmups": args.warmups, "peak_memory": None,
              "limitations": ["Includes parsing, layout, SVG encoding, and process/protocol costs.",
                              "Does not isolate engine costs or measure peak memory.",
                              "No native d2-rs source-to-SVG path exists yet."], "results": []}
    with tempfile.TemporaryDirectory() as scratch:
        output = Path(scratch) / "output.svg"
        for fixture in args.inputs:
            samples = {"tala": [], "weftan": []}
            for iteration in range(-args.warmups, args.runs):
                order = ("tala", "weftan") if iteration % 2 == 0 else ("weftan", "tala")
                for engine in order:
                    # Engine-specific flags differ; both receive the same explicit seed.
                    seed_flag = "--tala-seeds" if engine == "tala" else "--weftan-seeds"
                    cmd = [str(binary), "--layout=" + engine, seed_flag, str(args.seed),
                           "--omit-version", str(fixture.resolve()), str(output)]
                    start = time.perf_counter_ns()
                    completed = subprocess.run(cmd, env=environment, capture_output=True, timeout=120)
                    elapsed = (time.perf_counter_ns() - start) / 1e9
                    if completed.returncode:
                        raise RuntimeError(completed.stderr.decode(errors="replace"))
                    if iteration >= 0:
                        samples[engine].append(elapsed)
            result = {"fixture": str(fixture), "input_sha256": digest(fixture), "samples_s": samples,
                      "median_s": {engine: statistics.median(values) for engine, values in samples.items()}}
            result["weftan_over_tala"] = result["median_s"]["weftan"] / result["median_s"]["tala"]
            report["results"].append(result)
            print(json.dumps({k: v for k, v in result.items() if k != "samples_s"}), flush=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
