#!/usr/bin/env python3
"""Repeat exact D2 v0.9.0 and Rust port-swap traces for one fixed layout."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

from references import ROOT, digest, verify_source
from upstream_inputs import pristine_e2e


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


def side_schedule(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Retain the actual order of Go map iteration over port sides."""
    return [
        {key: event[key] for key in ("node", "side", "edges")}
        for event in events
        if event.get("event") == "swap-port-side-order"
        and event.get("phase") == "before-sort"
    ]


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture")
    parser.add_argument("--fixtures-dir", type=Path, default=pristine_e2e())
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.repeats < 2:
        parser.error("--repeats must be at least 2")

    verify_source()
    fixture = args.fixtures_dir / args.fixture
    if not fixture.is_file():
        parser.error(f"fixture not found: {fixture}")

    runs: list[dict[str, Any]] = []
    traces: dict[str, list[list[dict[str, Any]]]] = {"go": [], "rust": []}
    schedules: dict[str, list[list[dict[str, Any]]]] = {"go": [], "rust": []}
    for run_index in range(args.repeats):
        capture = subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts/capture-layout-trace.py"),
                args.fixture,
                "--fixtures-dir",
                str(args.fixtures_dir),
                "--seed",
                str(args.seed),
                "--swap-ports",
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        if capture.returncode:
            raise RuntimeError(
                f"capture failed on run {run_index + 1}:\n{capture.stdout}{capture.stderr}"
            )
        witness = Path(capture.stdout.strip())
        manifest = json.loads((witness / "manifest.json").read_text())
        run: dict[str, Any] = {
            "run": run_index + 1,
            "go_public_sha256": digest(witness / "go.stdout.json"),
            "rust_public_sha256": digest(witness / "rust.stdout.json"),
            "go_release_sha256": manifest["oracles"]["go"]["base_sha256"],
            "rust_release_sha256": manifest["oracles"]["rust"]["base_sha256"],
            "go_instrumentation_transparent": manifest["oracles"]["go"]["public_bytes_identical"],
            "rust_instrumentation_transparent": manifest["oracles"]["rust"]["public_bytes_identical"],
            "oracles": {},
        }
        for oracle in ("go", "rust"):
            events = json.loads((witness / f"{oracle}.swap-ports.json").read_text())
            traces[oracle].append(events)
            schedules[oracle].append(side_schedule(events))
            run["oracles"][oracle] = {
                "events": len(events),
                "trace_sha256": digest(witness / f"{oracle}.swap-ports.json"),
                "side_schedule_sha256": sha256_bytes(
                    json.dumps(schedules[oracle][-1], separators=(",", ":")).encode()
                ),
                "event_counts": manifest["oracles"][oracle].get("swap_port_event_counts", {}),
            }
        run["go_rust_trace_identical"] = traces["go"][-1] == traces["rust"][-1]
        run["public_outputs_identical"] = run["go_public_sha256"] == run["rust_public_sha256"]
        runs.append(run)
        print(
            f"captured run {run_index + 1}/{args.repeats}: "
            f"Go/Rust trace identical={run['go_rust_trace_identical']}, "
            f"public bytes identical={run['public_outputs_identical']}",
            flush=True,
        )

    pairwise: list[dict[str, Any]] = []
    for left_index in range(args.repeats):
        for right_index in range(left_index + 1, args.repeats):
            pair: dict[str, Any] = {
                "run_a": left_index + 1,
                "run_b": right_index + 1,
                "go_rust_trace_identical_in_run_a": runs[left_index]["go_rust_trace_identical"],
                "go_rust_trace_identical_in_run_b": runs[right_index]["go_rust_trace_identical"],
                "oracles": {},
            }
            for oracle in ("go", "rust"):
                pair["oracles"][oracle] = {
                    "trace_identical": traces[oracle][left_index] == traces[oracle][right_index],
                    "first_difference_path": first_difference_path(
                        traces[oracle][left_index], traces[oracle][right_index]
                    ),
                    "side_schedule_identical": schedules[oracle][left_index]
                    == schedules[oracle][right_index],
                    "side_schedule_first_difference_path": first_difference_path(
                        schedules[oracle][left_index], schedules[oracle][right_index]
                    ),
                }
            pairwise.append(pair)

    repeatable = {
        oracle: all(
            pair["oracles"][oracle]["trace_identical"] for pair in pairwise
        )
        for oracle in ("go", "rust")
    }
    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-tala-swap-port-trace-repeatability",
        "authority": "pristine D2 v0.9.0 with Go 1.27.0",
        "upstream_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "fixture": args.fixture,
        "fixture_sha256": digest(fixture),
        "seed": args.seed,
        "repeats": args.repeats,
        "normalization": "none; preserve event order, Go side-map traversal, and exact IEEE-754 route bits",
        "go_trace_repeatable": repeatable["go"],
        "rust_trace_repeatable": repeatable["rust"],
        "go_side_schedule_variants": len(
            {json.dumps(schedule, separators=(",", ":")) for schedule in schedules["go"]}
        ),
        "rust_side_schedule_variants": len(
            {json.dumps(schedule, separators=(",", ":")) for schedule in schedules["rust"]}
        ),
        "go_public_output_repeatable": len({run["go_public_sha256"] for run in runs}) == 1,
        "rust_public_output_repeatable": len({run["rust_public_sha256"] for run in runs}) == 1,
        "go_rust_public_outputs_identical_every_run": all(
            run["public_outputs_identical"] for run in runs
        ),
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
    print(
        f"Go/Rust trace repeatable: {report['go_trace_repeatable']}/"
        f"{report['rust_trace_repeatable']}"
    )
    print(
        f"side-order variants: Go {report['go_side_schedule_variants']}, "
        f"Rust {report['rust_side_schedule_variants']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
