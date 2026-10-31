#!/usr/bin/env python3
"""Merge strict TALA trace matrices without hiding known oracle variation."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


NONDETERMINISTIC_TRACE_EVIDENCE: dict[tuple[str, int], tuple[str, ...]] = {
    ("arrowhead_scaling.d2", 1): ("arrowhead-scaling-go-repeatability-seed1-20260923.json",),
    ("arrowhead_scaling.d2", 2): ("arrowhead-scaling-go-repeatability-seed2-20260923.json",),
    ("arrowhead_scaling.d2", 3): ("arrowhead-scaling-oracle-repeatability-seeds3-5-20260923.json",),
    ("arrowhead_scaling.d2", 5): ("arrowhead-scaling-oracle-repeatability-seeds3-5-20260923.json",),
    ("arrowhead_scaling.d2", 7): ("arrowhead-scaling-oracle-repeatability-seeds7-42-20260923.json",),
    ("arrowhead_scaling.d2", 42): ("arrowhead-scaling-oracle-repeatability-seeds7-42-20260923.json",),
    ("cross_arrowhead.d2", 1): ("cross-arrowhead-go-repeatability-seeds1-2-20260923.json",),
    ("cross_arrowhead.d2", 2): ("cross-arrowhead-go-repeatability-seeds1-2-20260923.json",),
    ("cross_arrowhead.d2", 3): ("cross-arrowhead-oracle-repeatability-seeds3-5-20260923.json",),
    ("cross_arrowhead.d2", 5): ("cross-arrowhead-oracle-repeatability-seeds3-5-20260923.json",),
    ("cross_arrowhead.d2", 7): ("cross-arrowhead-oracle-repeatability-seeds7-42-20260923.json",),
    ("cross_arrowhead.d2", 42): ("cross-arrowhead-oracle-repeatability-seeds7-42-20260923.json",),
    ("nested_layout_bug.d2", 2): ("nested-layout-bug-oracle-repeatability-seed2-20260923.json",),
    ("nested_layout_bug.d2", 5): ("nested-layout-bug-oracle-repeatability-seed5-20260923.json",),
    ("nested_layout_bug.d2", 42): ("nested-layout-bug-oracle-repeatability-seed42-20260923.json",),
    ("sequence-trace-order.d2", 1): (
        "tala-gap-sequence-trace-order-repeatability-seed1-after-restored-order-20260929.json",
    ),
    ("sequence-trace-order.d2", 2): (
        "tala-gap-sequence-trace-order-repeatability-seeds2-3-5-7-42-20260926.json",
    ),
    ("sequence-trace-order.d2", 3): (
        "tala-gap-sequence-trace-order-repeatability-seeds2-3-5-7-42-20260926.json",
    ),
    ("sequence-trace-order.d2", 5): (
        "tala-gap-sequence-trace-order-repeatability-seeds2-3-5-7-42-20260926.json",
    ),
    ("sequence-trace-order.d2", 7): (
        "tala-gap-sequence-trace-order-repeatability-seeds2-3-5-7-42-20260926.json",
    ),
    ("sequence-trace-order.d2", 42): (
        "tala-gap-sequence-trace-order-repeatability-seeds2-3-5-7-42-20260926.json",
    ),
}


def load_report(path: Path) -> dict[str, Any]:
    report = json.loads(path.read_text())
    if report.get("kind") != "d2-v0.9.0-tala-multiseed-trace-parity":
        raise ValueError(f"unsupported report kind in {path}")
    return report


def verifies_upstream_variation(path: Path, fixture: str, seed: int) -> bool:
    report = json.loads(path.read_text())
    if report.get("fixture") != fixture:
        return False
    result = next((item for item in report.get("results", []) if item.get("seed") == seed), None)
    if result is None:
        return False
    if report.get("kind") == "d2-v0.9.0-go-trace-repeatability":
        go = result
        go_gap = go["trace_families"]["gap"]
        public_stable = go["public_output_stable_across_runs"]
    elif report.get("kind") == "d2-v0.9.0-oracle-trace-repeatability":
        go = result["go"]
        go_gap = go["trace_families"]["gap"]
        public_stable = go["public_output_stable_across_runs"]
    else:
        return False
    return not go_gap["stable_across_runs"] and public_stable


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reports", nargs="+", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--evidence-dir",
        type=Path,
        default=Path("oracle-cache/reports"),
        help="directory containing repeatability evidence reports",
    )
    args = parser.parse_args()

    reports = [load_report(path) for path in args.reports]
    authority = reports[0]["authority"]
    commit = reports[0]["upstream_commit"]
    if any(report["authority"] != authority or report["upstream_commit"] != commit for report in reports):
        raise ValueError("reports do not share one pinned oracle authority")

    rows_by_key: dict[tuple[str, int], dict[str, Any]] = {}
    for report in reports:
        for original in report.get("results", []):
            key = (original["fixture"], int(original["seed"]))
            if key in rows_by_key:
                raise ValueError(f"duplicate fixture/seed result: {key}")
            row = dict(original)
            if row["status"] == "matched":
                row["trace_assessment"] = "strict_match"
            elif row["status"] == "inapplicable":
                row["trace_assessment"] = "upstream_inapplicable"
            elif key in NONDETERMINISTIC_TRACE_EVIDENCE:
                evidence = NONDETERMINISTIC_TRACE_EVIDENCE[key]
                missing = [name for name in evidence if not (args.evidence_dir / name).is_file()]
                if missing:
                    raise ValueError(f"missing nondeterminism evidence for {key}: {missing}")
                unsupported = [
                    name
                    for name in evidence
                    if not verifies_upstream_variation(
                        args.evidence_dir / name, key[0], key[1]
                    )
                ]
                if unsupported:
                    raise ValueError(f"evidence does not prove Go gap variation for {key}: {unsupported}")
                row["trace_assessment"] = "upstream_nondeterministic"
                row["trace_nondeterminism_evidence"] = [
                    (args.evidence_dir / name).as_posix() for name in evidence
                ]
            else:
                row["trace_assessment"] = "unclassified_divergence"
            rows_by_key[key] = row

    expected_fixtures = sorted({fixture for fixture, _ in rows_by_key})
    expected_seeds = sorted({seed for _, seed in rows_by_key})
    expected = {(fixture, seed) for fixture in expected_fixtures for seed in expected_seeds}
    if set(rows_by_key) != expected:
        missing = sorted(expected - set(rows_by_key))
        extra = sorted(set(rows_by_key) - expected)
        raise ValueError(f"matrix is incomplete or irregular; missing={missing}, extra={extra}")

    rows = [rows_by_key[(fixture, seed)] for seed in expected_seeds for fixture in expected_fixtures]
    strict_matches = sum(row["trace_assessment"] == "strict_match" for row in rows)
    upstream_nondeterministic = sum(
        row["trace_assessment"] == "upstream_nondeterministic" for row in rows
    )
    upstream_inapplicable = sum(
        row["trace_assessment"] == "upstream_inapplicable" for row in rows
    )
    unclassified = sum(row["trace_assessment"] == "unclassified_divergence" for row in rows)
    applicable_rows = [row for row in rows if row["status"] != "inapplicable"]
    applicable_public_bytes = sum(bool(row.get("public_bytes_identical")) for row in applicable_rows)
    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-tala-expanded-seed-trace-parity",
        "authority": authority,
        "upstream_commit": commit,
        "event_scope": reports[0].get("event_scope", "all emitted events"),
        "normalization": "strict event order and payload; no map-order normalization",
        "fixtures": expected_fixtures,
        "seeds": expected_seeds,
        "total": len(rows),
        "strict_trace_matches": strict_matches,
        "upstream_nondeterministic_trace_rows": upstream_nondeterministic,
        "upstream_inapplicable_rows": upstream_inapplicable,
        "unclassified_trace_divergences": unclassified,
        "applicable_rows": len(applicable_rows),
        "public_graph_exact": sum(bool(row.get("public_graph_exact")) for row in applicable_rows),
        "public_bytes_identical": applicable_public_bytes,
        "source_reports": [path.as_posix() for path in args.reports],
        "results": rows,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(
        f"strict={strict_matches}, upstream-nondeterministic={upstream_nondeterministic}, "
        f"inapplicable={upstream_inapplicable}, unclassified={unclassified}, "
        f"public-bytes={applicable_public_bytes}/{len(applicable_rows)}; "
        f"wrote {args.output}"
    )
    return int(unclassified != 0 or applicable_public_bytes != len(applicable_rows))


if __name__ == "__main__":
    raise SystemExit(main())
