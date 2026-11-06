#!/usr/bin/env python3
"""Write perf-bench's case selection to JSON for the WASM-vs-Go harness.

usage: export-cases.py <out.json> [--cases 12]
"""

import argparse
import importlib.util
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("out", type=Path)
    parser.add_argument("--cases", type=int, default=12)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location("perf_bench", ROOT / "scripts/perf-bench.py")
    perf_bench = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(perf_bench)
    cases = perf_bench.pick_cases(args.cases)
    args.out.write_text(json.dumps([{"name": name, "script": script} for _, name, script in cases]))
    print(f"{len(cases)} cases -> {args.out}")


if __name__ == "__main__":
    main()
