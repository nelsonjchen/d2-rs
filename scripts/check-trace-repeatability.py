#!/usr/bin/env python3
"""Measure repeatability of Go and Rust emitted traces for one fixed layout."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

from references import ROOT, digest, verify_source

FAMILIES = (
    "stages",
    "placement",
    "candidates",
    "scores",
    "packing",
    "routing",
    "rng",
    "gap",
    "connected",
)


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def difference_count(left: list[Any], right: list[Any]) -> int:
    return sum(
        left[index] != right[index]
        for index in range(max(len(left), len(right)))
        if index < len(left) and index < len(right)
    ) + abs(len(left) - len(right))


def first_difference_path(left: Any, right: Any, path: str = "$") -> str | None:
    if type(left) is not type(right):
        return path
    if isinstance(left, dict):
        for key in dict.fromkeys([*left, *right]):
            if key not in left or key not in right:
                return f"{path}.{key}"
            difference = first_difference_path(left[key], right[key], f"{path}.{key}")
            if difference:
                return difference
        return None
    if isinstance(left, list):
        for index, (left_item, right_item) in enumerate(zip(left, right)):
            difference = first_difference_path(left_item, right_item, f"{path}[{index}]")
            if difference:
                return difference
        return None if len(left) == len(right) else f"{path}.length"
    return None if left == right else path


def capture(fixture: str, fixtures_dir: Path, seed: int, connected_order: str) -> Path:
    subprocess.run(
        [
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
            "--connected-order",
            connected_order,
        ],
        cwd=ROOT,
        check=True,
    )
    return ROOT / "target/parity-witnesses" / f"{Path(fixture).stem}.seed-{seed}.gap.trace"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture")
    parser.add_argument("--fixtures-dir", type=Path, default=ROOT / "tests/d2")
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--connected-order", default="all")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.repeats < 2:
        parser.error("--repeats must be at least 2")

    verify_source()
    fixture_path = args.fixtures_dir / args.fixture
    if not fixture_path.is_file():
        parser.error(f"fixture not found: {fixture_path}")

    runs: list[dict[str, Any]] = []
    captured: list[dict[str, Any]] = []
    for run_index in range(args.repeats):
        witness = capture(args.fixture, args.fixtures_dir, args.seed, args.connected_order)
        manifest = json.loads((witness / "manifest.json").read_text())
        run: dict[str, Any] = {
            "run": run_index + 1,
            "go_public_sha256": digest(witness / "go.stdout.json"),
            "rust_public_sha256": digest(witness / "rust.stdout.json"),
            "go_instrumentation_transparent": manifest["oracles"]["go"]["public_bytes_identical"],
            "rust_instrumentation_transparent": manifest["oracles"]["rust"]["public_bytes_identical"],
            "go_trace_sha256": manifest["oracles"]["go"]["sha256"],
            "rust_trace_sha256": manifest["oracles"]["rust"]["sha256"],
            "go_release_sha256": manifest["oracles"]["go"]["base_sha256"],
            "rust_release_sha256": manifest["oracles"]["rust"]["base_sha256"],
            "event_families": {},
        }
        events_for_run: dict[str, dict[str, list[dict[str, Any]]]] = {}
        for oracle in ("go", "rust"):
            events_for_run[oracle] = {}
            for family in FAMILIES:
                path = witness / f"{oracle}.{family}.json"
                events = json.loads(path.read_text())
                events_for_run[oracle][family] = events
                run["event_families"].setdefault(family, {})[oracle] = {
                    "events": len(events),
                    "sha256": digest(path),
                }
        captured.append(events_for_run)
        runs.append(run)

    pairwise: list[dict[str, Any]] = []
    for left_index in range(len(captured)):
        for right_index in range(left_index + 1, len(captured)):
            row: dict[str, Any] = {
                "run_a": left_index + 1,
                "run_b": right_index + 1,
                "go_public_identical": runs[left_index]["go_public_sha256"]
                == runs[right_index]["go_public_sha256"],
                "rust_public_identical": runs[left_index]["rust_public_sha256"]
                == runs[right_index]["rust_public_sha256"],
                "event_families": {},
            }
            for family in FAMILIES:
                comparison: dict[str, Any] = {}
                for oracle in ("go", "rust"):
                    left = captured[left_index][oracle][family]
                    right = captured[right_index][oracle][family]
                    comparison[oracle] = {
                        "identical": left == right,
                        "event_count_delta": len(left) - len(right),
                        "different_event_count": difference_count(left, right),
                        "first_difference_path": first_difference_path(left, right),
                    }
                row["event_families"][family] = comparison
            pairwise.append(row)

    nondeterministic = {
        oracle: [
            family
            for family in FAMILIES
            if any(
                not pair["event_families"][family][oracle]["identical"]
                for pair in pairwise
            )
        ]
        for oracle in ("go", "rust")
    }
    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-tala-trace-repeatability",
        "authority": "pristine D2 v0.9.0 with Go 1.27.0",
        "upstream_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "fixture": args.fixture,
        "fixture_sha256": digest(fixture_path),
        "seed": args.seed,
        "repeats": args.repeats,
        "connected_order_target": args.connected_order,
        "normalization": "none; repeated Go and Rust traces are compared exactly, preserving event and map iteration order",
        "nondeterministic_event_families": nondeterministic,
        "go_public_output_repeatable": len({run["go_public_sha256"] for run in runs}) == 1,
        "rust_public_output_repeatable": len({run["rust_public_sha256"] for run in runs}) == 1,
        "instrumentation_transparent_every_run": all(
            run["go_instrumentation_transparent"]
            and run["rust_instrumentation_transparent"]
            for run in runs
        ),
        "runs": runs,
        "pairwise": pairwise,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"wrote {args.output}")
    print(f"nondeterministic Go families: {nondeterministic['go']}")
    print(f"nondeterministic Rust families: {nondeterministic['rust']}")
    print(
        "Go/Rust public outputs repeatable: "
        f"{report['go_public_output_repeatable']}/{report['rust_public_output_repeatable']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
