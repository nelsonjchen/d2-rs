#!/usr/bin/env python3
"""Audit repeated direct-engine traces for one pinned GraphJSON fixture."""

from __future__ import annotations

import argparse
import collections
import hashlib
import importlib.util
import json
import sys
from pathlib import Path
from typing import Any

from references import ROOT, REFERENCE, digest, verify_oracle, verify_source, read_report


def load_checker():
    checker_path = ROOT / "scripts/check-graphjson-trace.py"
    spec = importlib.util.spec_from_file_location("check_graphjson_trace", checker_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load trace checker: {checker_path}")
    checker = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = checker
    spec.loader.exec_module(checker)
    return checker


def json_events(stderr: bytes) -> list[dict[str, Any]]:
    events = []
    for line in stderr.decode(errors="replace").splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and isinstance(value.get("event"), str):
            events.append(value)
    return events


def sha256_json(value: Any) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def summarize_runs(
    runs: list[dict[str, Any]], implementation: str, families: list[str]
) -> dict[str, Any]:
    outputs = [run[f"{implementation}_stdout_sha256"] for run in runs]
    trace_hashes = [run[f"{implementation}_trace_sha256"] for run in runs]
    family_summary = {}
    for family in families:
        family_runs = [run[f"{implementation}_families"][family] for run in runs]
        hashes = [row["sha256"] for row in family_runs]
        family_summary[family] = {
            "stable_across_runs": len(set(hashes)) == 1,
            "unique_trace_hashes": len(set(hashes)),
            "event_counts": [row["event_count"] for row in family_runs],
            "sha256": hashes,
        }
    return {
        "public_output_stable_across_runs": len(set(outputs)) == 1,
        "public_output_sha256": outputs,
        "normalized_trace_stable_across_runs": len(set(trace_hashes)) == 1,
        "normalized_trace_sha256": trace_hashes,
        "trace_families": family_summary,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", help="upstream-relative GraphJSON fixture path")
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument(
        "--rust-binary",
        type=Path,
        default=ROOT / "target/diagnostic/examples/graphjson-layout",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "oracle-cache/reports/graphjson-trace-repeatability.json",
    )
    args = parser.parse_args()
    if args.runs < 2:
        parser.error("--runs must be at least 2")

    checker = load_checker()
    source = verify_source()
    fixture = (source / args.fixture).resolve()
    try:
        fixture.relative_to(source)
    except ValueError as error:
        raise SystemExit("fixture path must remain inside the pinned D2 source") from error
    inventory = read_report("engine-fixture-inventory.json", "scripts/audit-engine-fixtures.py")
    inventory_rows = {
        row["upstream_path"]: row
        for row in inventory["fixtures"]
        if row.get("format") == "layout_graph_json"
    }
    inventory_row = inventory_rows.get(args.fixture)
    if inventory_row is None:
        raise SystemExit(f"fixture is not in the pinned GraphJSON inventory: {args.fixture}")
    fixture_sha256 = digest(fixture)
    if inventory_row["sha256"] != fixture_sha256:
        raise SystemExit(f"fixture checksum mismatch: {fixture}")

    go = REFERENCE / "bin/tala-engine-oracle-trace"
    go_provenance = verify_oracle(go, trace=True)
    if not args.rust_binary.is_file():
        raise SystemExit(f"missing diagnostic GraphJSON binary: {args.rust_binary}")

    families = sorted(checker.FULL_TRACE_EVENTS)
    runs = []
    traces_by_run = []
    for run_index in range(1, args.runs + 1):
        run = {"run": run_index}
        traces = {}
        for implementation, binary, command_args, env_key in (
            ("go", go, ["-seed", str(args.seed)], "TALA_TRACE_JSONL"),
            ("rust", args.rust_binary, [str(args.seed)], "WEFTAN_TRACE_JSONL"),
        ):
            exit_code, stdout, stderr = checker.run(
                [str(binary), *command_args],
                fixture.read_bytes(),
                env_key,
                args.timeout,
                include_full_trace=True,
            )
            raw_events = json_events(stderr)
            trace = checker.events(
                stderr,
                include_full_trace=True,
            )
            counts = collections.Counter(event["event"] for event in raw_events)
            run[f"{implementation}_exit"] = exit_code
            run[f"{implementation}_stdout_sha256"] = hashlib.sha256(stdout).hexdigest()
            run[f"{implementation}_stdout_bytes"] = len(stdout)
            run[f"{implementation}_raw_event_counts"] = dict(sorted(counts.items()))
            run[f"{implementation}_uncompared_json_event_counts"] = dict(
                sorted(
                    (name, count)
                    for name, count in counts.items()
                    if name not in checker.FULL_TRACE_EVENTS
                )
            )
            run[f"{implementation}_trace_sha256"] = checker.trace_sha256(trace)
            run[f"{implementation}_families"] = {}
            for family in families:
                family_events = [event for event in trace if event["event"] == family]
                run[f"{implementation}_families"][family] = {
                    "event_count": len(family_events),
                    "sha256": sha256_json(family_events),
                }
            traces[implementation] = trace
        run["cross_implementation_trace_match"] = (
            checker.first_event_difference(traces["go"], traces["rust"]) is None
        )
        traces_by_run.append(traces)
        runs.append(run)
        print(
            f"run {run_index}: {run['cross_implementation_trace_match'] and 'MATCH' or 'DIVERGE'}",
            flush=True,
        )

    summaries = {
        implementation: summarize_runs(runs, implementation, families)
        for implementation in ("go", "rust")
    }
    run_variable_families = sorted(
        family
        for family in families
        if not summaries["go"]["trace_families"][family]["stable_across_runs"]
        or not summaries["rust"]["trace_families"][family]["stable_across_runs"]
    )
    for run, traces in zip(runs, traces_by_run):
        difference = checker.first_event_difference(traces["go"], traces["rust"])
        run["cross_implementation_trace_match_excluding_run_variable_families"] = (
            checker.first_event_difference(
                [event for event in traces["go"] if event["event"] not in run_variable_families],
                [event for event in traces["rust"] if event["event"] not in run_variable_families],
            )
            is None
        )
        if difference is not None:
            run["first_cross_implementation_difference"] = {
                "event_index": difference["event_index"],
                "difference": difference["difference"],
                "go_event": difference.get("go_event", {}).get("event"),
                "rust_event": difference.get("rust_event", {}).get("event"),
            }

    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-graphjson-trace-repeatability",
        "authority": "pinned D2 v0.9.0 source build with Go 1.27.0",
        "upstream_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "fixture": args.fixture,
        "fixture_sha256": fixture_sha256,
        "seed": args.seed,
        "runs": runs,
        "run_variable_event_families": run_variable_families,
        "cross_implementation_exact_match_excluding_only_run_variable_families": all(
            run["cross_implementation_trace_match_excluding_run_variable_families"]
            for run in runs
        ),
        "go_oracle_sha256": digest(go),
        "trace_patch_sha256": go_provenance["trace_patch_sha256"],
        "rust_binary": str(args.rust_binary.resolve()),
        "rust_binary_sha256": digest(args.rust_binary),
        "go": summaries["go"],
        "rust": summaries["rust"],
        "strict_event_order": True,
        "normalization": [
            "same normalization as check-graphjson-trace.py --include-full-trace",
            "preserve every compared event family and its source order",
            "hash each event family without sorting events or array payloads",
        ],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"wrote {args.output}")
    return int(any(run["go_exit"] or run["rust_exit"] for run in runs))


if __name__ == "__main__":
    raise SystemExit(main())
