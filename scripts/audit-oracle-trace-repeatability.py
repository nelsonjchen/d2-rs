#!/usr/bin/env python3
"""Capture repeated Go and Rust oracle traces and report run-to-run variation."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from collections import Counter
from pathlib import Path
from typing import Any

from references import ROOT, digest, verify_source


FAMILIES = ("stages", "placement", "candidates", "scores", "packing", "routing", "rng", "gap")


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def capture(fixture: str, fixtures_dir: Path, seed: int) -> Path:
    command = [
        sys.executable,
        str(ROOT / "scripts/capture-layout-trace.py"),
        fixture,
        "--fixtures-dir",
        str(fixtures_dir),
        "--seed",
        str(seed),
        "--placement",
        "--packing",
        "--gap",
        "--routing",
        "--rng",
    ]
    subprocess.run(command, cwd=ROOT, check=True)
    suffix = f"{Path(fixture).stem}.seed-{seed}.gap.trace"
    return ROOT / "target/parity-witnesses" / suffix


def run_summary(
    witness: Path, run_index: int
) -> tuple[dict[str, Any], dict[str, dict[str, list[dict[str, Any]]]]]:
    manifest = json.loads((witness / "manifest.json").read_text())
    events: dict[str, dict[str, list[dict[str, Any]]]] = {}
    summaries: dict[str, dict[str, Any]] = {}
    for implementation in ("go", "rust"):
        events[implementation] = {}
        family_summary = {}
        for family in FAMILIES:
            path = witness / f"{implementation}.{family}.json"
            events[implementation][family] = json.loads(path.read_text())
            family_summary[family] = {
                "event_count": len(events[implementation][family]),
                "sha256": file_sha256(path),
            }
        stdout_path = witness / f"{implementation}.stdout.json"
        summaries[implementation] = {
            "public_output_sha256": file_sha256(stdout_path),
            "public_output_bytes": stdout_path.stat().st_size,
            "public_output_matches_release": manifest["oracles"][implementation][
                "public_bytes_identical"
            ],
            "trace_families": family_summary,
        }
    summary = {
        "run": run_index,
        **summaries,
    }
    return summary, events


def compare_runs(
    runs: list[dict[str, Any]],
    events: list[dict[str, dict[str, list[dict[str, Any]]]]],
    implementation: str,
) -> dict[str, Any]:
    family_results = {}
    for family in FAMILIES:
        digests = [run[implementation]["trace_families"][family]["sha256"] for run in runs]
        family_results[family] = {
            "stable_across_runs": len(set(digests)) == 1,
            "unique_trace_hashes": len(set(digests)),
            "event_counts": [
                run[implementation]["trace_families"][family]["event_count"]
                for run in runs
            ],
            "comparisons_to_run_1": [
                {
                    "run": index + 1,
                    **differing_event_summary(
                        events[0][implementation][family],
                        events[index][implementation][family],
                    ),
                }
                for index in range(1, len(events))
                if events[index][implementation][family]
                != events[0][implementation][family]
            ],
        }
    output_hashes = [run[implementation]["public_output_sha256"] for run in runs]
    return {
        "public_output_stable_across_runs": len(set(output_hashes)) == 1,
        "trace_families": family_results,
    }


def differing_event_summary(
    first: list[dict[str, Any]], other: list[dict[str, Any]]
) -> dict[str, Any]:
    differing_indices = []
    changed_fields: Counter[str] = Counter()
    first_path = None
    for index in range(max(len(first), len(other))):
        left = first[index] if index < len(first) else None
        right = other[index] if index < len(other) else None
        if left == right:
            continue
        differing_indices.append(index)
        if isinstance(left, dict) and isinstance(right, dict):
            for key in set(left) | set(right):
                if key not in left or key not in right or left[key] != right[key]:
                    changed_fields[key] += 1
                    if first_path is None:
                        first_path = f"$[{index}].{key}"
        elif first_path is None:
            first_path = f"$[{index}]"
    return {
        "different_event_count_vs_run_1": len(differing_indices),
        "first_different_event_index_vs_run_1": differing_indices[0] if differing_indices else None,
        "first_difference_path_vs_run_1": first_path,
        "changed_top_level_fields_vs_run_1": dict(sorted(changed_fields.items())),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture")
    parser.add_argument("--fixtures-dir", type=Path, default=ROOT / "tests/d2")
    parser.add_argument("--seed", action="append", required=True, type=int, dest="seeds")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--fail-on-nondeterminism", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.runs < 2:
        parser.error("--runs must be at least 2")

    verify_source()
    fixtures_dir = args.fixtures_dir.resolve()
    fixture_path = fixtures_dir / args.fixture
    if not fixture_path.is_file():
        raise SystemExit(f"fixture not found: {fixture_path}")

    results = []
    for seed in args.seeds:
        run_summaries = []
        run_events = []
        for run_index in range(1, args.runs + 1):
            witness = capture(args.fixture, fixtures_dir, seed)
            summary, events = run_summary(witness, run_index)
            run_summaries.append(summary)
            run_events.append(events)
        go_results = compare_runs(run_summaries, run_events, "go")
        rust_results = compare_runs(run_summaries, run_events, "rust")
        last_manifest = json.loads((witness / "manifest.json").read_text())
        results.append(
            {
                "seed": seed,
                "runs": run_summaries,
                "go": go_results,
                "rust": rust_results,
            }
        )

    report = {
        "schema_version": 2,
        "kind": "d2-v0.9.0-oracle-trace-repeatability",
        "fixture": args.fixture,
        "fixture_sha256": digest(fixture_path),
        "authority": "pinned D2 v0.9.0 source build with Go 1.27.0",
        "go_trace_oracle_sha256": last_manifest["oracles"]["go"]["sha256"],
        "rust_trace_oracle_sha256": last_manifest["oracles"]["rust"]["sha256"],
        "official_release_sha256": last_manifest["oracles"]["go"]["base_sha256"],
        "rust_release_sha256": last_manifest["oracles"]["rust"]["base_sha256"],
        "strict_ordering": True,
        "results": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"wrote {args.output}")
    nondeterministic = (
        any(
            not result[implementation]["public_output_stable_across_runs"]
            for result in results
            for implementation in ("go", "rust")
        )
        or any(
            not family["stable_across_runs"]
            for result in results
            for implementation in ("go", "rust")
            for family in result[implementation]["trace_families"].values()
        )
    )
    return int(nondeterministic and args.fail_on_nondeterminism)


if __name__ == "__main__":
    raise SystemExit(main())
