#!/usr/bin/env python3
"""Check default multi-seed selection against the pinned D2 v0.9.0 oracle."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

from references import ROOT, REFERENCE, digest, verify_oracle, verify_source

DEFAULT_FIXTURES = (
    "sequence-trace-order.d2",
    "all_shapes_link.d2",
    "nested-lock.d2",
    "retained-grid-cluster-gap.d2",
    "nested-direction-grid.d2",
    "right-near.d2",
)


def run(command: list[str], data: bytes) -> bytes:
    result = subprocess.run(command, cwd=ROOT, input=data, capture_output=True)
    if result.returncode:
        raise RuntimeError(
            f"command failed ({result.returncode}): {' '.join(command)}\n"
            f"stderr:\n{result.stderr.decode(errors='replace')}"
        )
    return result.stdout


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--fixture",
        action="append",
        dest="fixtures",
        help="fixture to check; may be repeated (defaults to six seed-selection regressions)",
    )
    parser.add_argument(
        "--report", type=Path, default=ROOT / "oracle-cache/reports/seed-selection-parity.json"
    )
    parser.add_argument(
        "--fixtures-dir",
        type=Path,
        default=ROOT / "tests/d2",
        help="directory containing the requested .d2 fixtures",
    )
    args = parser.parse_args()

    verify_source()
    oracle = REFERENCE / "bin/d2-oracle"
    verify_oracle(oracle)
    rust = ROOT / "target/release/examples/tala-oracle"
    report_probe = ROOT / "target/release/examples/tala-seed-report"
    fixture_names = args.fixtures or list(DEFAULT_FIXTURES)
    fixture_root = args.fixtures_dir
    if not fixture_root.is_absolute():
        fixture_root = ROOT / fixture_root
    rows = []
    for fixture_name in fixture_names:
        fixture = fixture_root / fixture_name
        measured = run([str(oracle), "measure", "--path", fixture.name], fixture.read_bytes())

        oracle_multi = run([str(oracle), "layout", "--tala-seeds", "1,2,3"], measured)
        oracle_winner_seeds = [
            seed
            for seed in (1, 2, 3)
            if run([str(oracle), "layout", "--tala-seeds", str(seed)], measured)
            == oracle_multi
        ]
        rust_multi = run([str(rust), "layout", "--tala-seeds", "1,2,3"], measured)
        rust_seed_report = json.loads(
            run([str(report_probe), "--tala-seeds", "1,2,3"], measured)
        )

        if not oracle_winner_seeds:
            raise SystemExit(
                f"{fixture_name}: pinned Go multi-seed result does not match a single-seed output"
            )
        if rust_seed_report["selected_seed"] not in oracle_winner_seeds:
            raise SystemExit(
                f"{fixture_name}: Rust selected seed {rust_seed_report['selected_seed']}, "
                f"but Go's output matches seeds {oracle_winner_seeds}"
            )
        if rust_multi != oracle_multi:
            raise SystemExit(
                f"{fixture_name}: Rust multi-seed output differs from pinned D2 v0.9.0"
            )

        rows.append({
            "fixture": fixture_name,
            "oracle_selected_seeds_matching_output": oracle_winner_seeds,
            "rust_selected_seed": rust_seed_report["selected_seed"],
            "oracle_multi_matches_single_seed": True,
            "rust_multi_equals_oracle": True,
            "measured_sha256": hashlib.sha256(measured).hexdigest(),
            "output_sha256": hashlib.sha256(oracle_multi).hexdigest(),
            "candidate_scores": rust_seed_report["candidates"],
        })
        print(f"{fixture_name}: MATCH seed {rust_seed_report['selected_seed']}", flush=True)

    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-tala-seed-selection-matrix",
        "authority": "pinned D2 v0.9.0 source build with Go 1.27.0",
        "upstream_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "fixtures_dir": str(fixture_root.relative_to(ROOT))
        if fixture_root.is_relative_to(ROOT)
        else str(fixture_root),
        "seeds": [1, 2, 3],
        "oracle_sha256": digest(oracle),
        "rust_release_sha256": digest(rust),
        "rust_seed_report_sha256": digest(report_probe),
        "matched": len(rows),
        "total": len(fixture_names),
        "results": rows,
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(f"MATCH {len(rows)}/{len(fixture_names)} exact multi-seed outputs; wrote {args.report}")


if __name__ == "__main__":
    main()
