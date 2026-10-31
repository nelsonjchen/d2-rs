#!/usr/bin/env python3
"""Check implemented boundaries, reporting strict first differences.

This is deliberately not the old stage/coordinate-normalizing trace comparator.
"""
import argparse
import json
from pathlib import Path
import random
import subprocess

from exact_json import decode_output, difference

from references import ROOT, REFERENCE, MANIFEST, verify_source, digest


def run(command, source):
    try:
        result = subprocess.run([str(arg) for arg in command], input=source, capture_output=True, timeout=120)
    except subprocess.TimeoutExpired:
        return {"harness_timeout": True}
    if result.returncode:
        return {"error": result.stderr.decode(errors="replace").strip()}
    return {"output": decode_output(result.stdout)}



def execution_difference(left, right):
    # Two timed-out processes establish no result parity.
    if "harness_timeout" in left or "harness_timeout" in right:
        return {"harness_timeout": True}
    return difference(left, right)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "oracle-cache/reports/boundary-baseline.json")
    parser.add_argument("--fixtures-dir", type=Path, default=ROOT / "tests/d2")
    parser.add_argument("--seeds", type=int, nargs="+", default=[1, 2, 3])
    parser.add_argument("--layout-only", action="store_true")
    args = parser.parse_args()
    source = verify_source()
    oracle = REFERENCE / "bin/d2-oracle"
    rust_position = ROOT / "target/release/examples/position-oracle"
    rust_layout = ROOT / "target/release/examples/tala-oracle"
    results = []
    generator = random.Random(900)
    alphabet = "a\n\r\tè𐀀🦀界\u0301\x00"
    inputs = ["", "a\nè𐀀", "\r\n", "👩\u200d💻"]
    inputs += ["".join(generator.choices(alphabet, k=generator.randrange(1, 100))) for _ in range(40)]
    for utf16 in (() if args.layout_only else (False, True)):
        for index, source in enumerate(inputs):
            flags = ["--utf16"] if utf16 else []
            left = run([oracle, "position", *flags], source.encode())
            right = run([rust_position, *flags], source.encode())
            results.append({"boundary": "position", "case": index, "utf16": utf16,
                            "difference": execution_difference(left, right)})
    fixtures = sorted(args.fixtures_dir.glob("*.d2"))
    if not fixtures:
        raise RuntimeError(f"No D2 fixtures found in {args.fixtures_dir}")
    for fixture in fixtures:
        try:
            measured = subprocess.run([str(oracle), "measure", "--path", fixture.name],
                                      input=fixture.read_bytes(), capture_output=True, timeout=120)
        except subprocess.TimeoutExpired:
            results.append({"boundary": "layout", "fixture": fixture.name,
                            "difference": {"oracle_measure_timeout": True}})
            continue
        if measured.returncode:
            results.append({"boundary": "layout", "fixture": fixture.name,
                            "applicable": False,
                            "outcome": "oracle_measure_error",
                            "difference": None,
                            "reason": "The v0.9.0 measurement phase rejects this source before a measured graph exists; source-render error parity is checked separately."})
            continue
        for seed in args.seeds:
            left = run([oracle, "layout", "--tala-seeds", str(seed)], measured.stdout)
            right = run([rust_layout, "layout", "--tala-seeds", str(seed)], measured.stdout)
            results.append({"boundary": "layout", "fixture": fixture.name, "seed": seed,
                            "oracle_outcome": next(iter(left)),
                            "rust_outcome": next(iter(right)),
                            **({"matched_error": left["error"]} if "error" in left and left == right else {}),
                            "difference": execution_difference(left, right)})
    report = {"oracle_sha256": digest(oracle), "rust_layout_sha256": digest(rust_layout),
              "rust_position_sha256": digest(rust_position),
              "comparison": "ordered arrays, exact numeric bits, all JSON fields; key order ignored",
              "results": results,
              "successful_layouts": sum(row.get("boundary") == "layout" and
                                        row.get("oracle_outcome") == "output" and
                                        row.get("rust_outcome") == "output" and
                                        row["difference"] is None for row in results),
              "passed": sum(row["difference"] is None for row in results),
              "failed": sum(row["difference"] is not None for row in results),
              "inapplicable": sum(row.get("applicable") is False for row in results)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k != "results"}, indent=2))
    raise SystemExit(bool(report["failed"]))


if __name__ == "__main__":
    main()
