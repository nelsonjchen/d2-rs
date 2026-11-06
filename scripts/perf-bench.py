#!/usr/bin/env python3
"""Compare d2-rs and the pinned Go D2 oracle on wall time, CPU and peak RSS.

Inputs are e2e case sources from references.E2E_CASES, picked by size so the
corpus is fixed: the largest N cases plus a spread of smaller ones. Each
(case, engine, implementation) runs --repeat times; medians are reported.
Outputs are compared byte for byte, so a speed-up can never hide a parity
change. Writes a JSON report and prints a per-engine summary.

usage: perf-bench.py --rust <d2-rs> --output <report.json> [--cases 12]
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import resource
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import references

ENGINES = ("dagre", "elk", "tala")
REMOTE_ICON = re.compile(r"^[^#\n]*\bicon\s*:\s*\"?https?://", re.M)


def pick_cases(count):
    cases = []
    for path in sorted(references.E2E_CASES.glob("*.json")):
        case = json.loads(path.read_text())
        script = case.get("script") or ""
        # Remote icons are fetched for measurement: that times the network,
        # not d2, and sandboxed lanes have no network at all.
        if script.strip() and not REMOTE_ICON.search(script):
            cases.append((len(script), case["name"], script))
    cases.sort()
    largest = cases[-(count // 2):]
    step = max(1, (len(cases) - len(largest)) // max(1, count - len(largest)))
    spread = cases[: len(cases) - len(largest): step][: count - len(largest)]
    return spread + largest


def run(binary, engine, source, out, extra=()):
    # Isolate getrusage per invocation: RUSAGE_CHILDREN's maximum RSS is a
    # high-water mark, so measuring all binaries in this process is incorrect.
    # /usr/bin/time -l also requires sysctl privileges unavailable in managed
    # macOS sandboxes. The worker needs only wait4's ordinary child usage.
    out.unlink(missing_ok=True)
    result = subprocess.run(
        [sys.executable, str(Path(__file__).resolve()), "--measure-child",
         str(binary.resolve()), f"--layout={engine}", *extra, str(source), str(out)],
        capture_output=True, text=True, timeout=610, check=False,
    )
    if result.returncode:
        raise RuntimeError(result.stderr)
    return tuple(json.loads(result.stdout))


def measure_child(command):
    start = time.perf_counter()
    # A stderr pipe lets communicate wait for EOF. With both streams sent to
    # DEVNULL, Popen.wait(timeout) polls and rounds short timings up to 50 ms.
    result = subprocess.run(command, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=600, check=False)
    wall = time.perf_counter() - start
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    rss = usage.ru_maxrss if sys.platform == "darwin" else usage.ru_maxrss * 1024
    print(json.dumps([result.returncode, wall, usage.ru_utime + usage.ru_stime, rss]))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--go", type=Path, default=references.GO_ORACLE)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cases", type=int, default=12)
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--warmup", type=int, default=1)
    args = parser.parse_args()
    rows = []
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        for size, name, script in pick_cases(args.cases):
            source = tmp / "input.d2"
            source.write_text(script)
            for engine in ENGINES:
                row = {"case": name, "bytes": size, "engine": engine}
                outputs = {}
                impls = (("go", args.go), ("rust", args.rust))
                samples = {impl: [] for impl, _ in impls}
                hashes = {impl: [] for impl, _ in impls}
                # Interleave implementations per repeat (after one untimed
                # warm-up each) so host noise lands on both sides alike.
                for index in range(args.warmup + args.repeat):
                    for impl, binary in impls:
                        out = tmp / f"{impl}.svg"
                        sample = run(binary, engine, source, out)
                        if index < args.warmup:
                            continue
                        samples[impl].append(sample)
                        hashes[impl].append(hashlib.sha256(out.read_bytes()).hexdigest() if out.exists() else None)
                        outputs[impl] = out.read_bytes() if out.exists() else None
                for impl, _ in impls:
                    row[impl] = {
                        "exit": samples[impl][-1][0],
                        "sample_exits": [sample[0] for sample in samples[impl]],
                        "output_hashes": hashes[impl],
                        "wall": statistics.median(s[1] for s in samples[impl]),
                        "cpu": statistics.median(s[2] for s in samples[impl]),
                        "rss": statistics.median(s[3] for s in samples[impl]),
                    }
                row["identical"] = (outputs["go"] is not None and outputs["go"] == outputs["rust"]
                                    and all(row[impl]["sample_exits"] == [0] * args.repeat
                                            and len(set(row[impl]["output_hashes"])) == 1
                                            for impl in ("go", "rust")))
                rows.append(row)
    summary = {}
    for engine in ENGINES:
        sel = [r for r in rows if r["engine"] == engine and r["go"]["exit"] == 0]
        if not sel:
            continue
        ratio = lambda key, selected=sel: statistics.median(r["rust"][key] / max(r["go"][key], 1e-3) for r in selected)
        # The median hides a bimodal split (Go pays ~25 ms runtime startup on
        # small boards; large boards are compute-bound), so also report the
        # geometric mean and the summed time.
        geomean = lambda key, selected=sel: statistics.geometric_mean(r["rust"][key] / max(r["go"][key], 1e-3) for r in selected)
        total = lambda key, selected=sel: sum(r["rust"][key] for r in selected) / max(sum(r["go"][key] for r in selected), 1e-3)
        summary[engine] = {
            "cases": len(sel),
            "wall_ratio": round(ratio("wall"), 2),
            "cpu_ratio": round(ratio("cpu"), 2),
            "rss_ratio": round(ratio("rss"), 2),
            "wall_geomean": round(geomean("wall"), 2),
            "wall_total_ratio": round(total("wall"), 2),
            "cpu_total_ratio": round(total("cpu"), 2),
            "identical": sum(r["identical"] for r in sel),
        }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps({"source_commit": references.SOURCE_COMMIT,
                                     "measurement": "isolated child getrusage; perf_counter wall seconds; RSS bytes",
                                     "binaries": {impl: {"path": str(binary.resolve()), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
                                                  for impl, binary in (("go", args.go), ("rust", args.rust))},
                                     "summary": summary, "rows": rows}, indent=1) + "\n")
    for engine, s in summary.items():
        print(f"{engine}: rust/go wall {s['wall_ratio']}x (geomean {s['wall_geomean']}x, total {s['wall_total_ratio']}x) cpu {s['cpu_ratio']}x (total {s['cpu_total_ratio']}x) rss {s['rss_ratio']}x; identical {s['identical']}/{s['cases']}")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--measure-child":
        measure_child(sys.argv[2:])
    else:
        main()
