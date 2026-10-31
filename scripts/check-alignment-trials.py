#!/usr/bin/env python3
"""Compare AlignAxes ConnectedNodeSet trials from pinned Go and Rust oracles.

The checker replays measured GraphJSON already retained by capture-layout-trace.py.
It compares trial order, edge identity, ordered move-set identities, rejection or
acceptance, and exact IEEE-754 score/translation bits. It also verifies that
diagnostic hooks leave each oracle's public output unchanged.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import struct
import subprocess
from pathlib import Path
from typing import Any

from references import MANIFEST, REFERENCE, ROOT, digest, verify_oracle, verify_source


FIXTURES = (
    "retained-grid-cluster-gap",
    "nesting_power",
    "queue_workers",
    "teleport_grid",
)
GO_TRIAL = re.compile(
    r"^ALIGN_TRIAL_GO edge=(\d+)>(\d+) side=(from|to) score=([^ ]+) "
    r"dx=([^ ]+) dy=([^ ]+) nodes=(.*)$"
)
RUST_TRIAL = re.compile(
    r"^ALIGN_TRIAL_RUST edge=(\d+)>(\d+) side=(from|to) base=([^ ]+) "
    r"delta=([^,]+),([^ ]+) result="
    r"(None|Some\(\(([^,]+), Point \{ x: ([^,]+), y: ([^}]+) \}\)\)) "
    r"nodes=(.*)$"
)


def f64_bits(value: str) -> str:
    return struct.pack(">d", float(value)).hex()


def node_ids(value: str) -> list[str]:
    return [node for node in value.split(",") if node]


def parse_go(stderr: bytes) -> list[dict[str, Any]]:
    events = []
    for line in stderr.decode("utf-8", errors="replace").splitlines():
        if not line.startswith("ALIGN_TRIAL_GO "):
            continue
        match = GO_TRIAL.match(line)
        if match is None:
            raise ValueError(f"could not parse Go alignment event: {line}")
        source, target, side, score, dx, dy, nodes = match.groups()
        events.append(
            {
                "edge": [source, target],
                "side": side,
                "nodes": node_ids(nodes),
                "result": None
                if float(score) == float("inf")
                else {
                    "score_bits": f64_bits(score),
                    "dx_bits": f64_bits(dx),
                    "dy_bits": f64_bits(dy),
                },
            }
        )
    return events


def parse_rust(stderr: bytes) -> list[dict[str, Any]]:
    events = []
    for line in stderr.decode("utf-8", errors="replace").splitlines():
        if not line.startswith("ALIGN_TRIAL_RUST "):
            continue
        match = RUST_TRIAL.match(line)
        if match is None:
            raise ValueError(f"could not parse Rust alignment event: {line}")
        source, target, side, _base, _dx, _dy, result, score, dx, dy, nodes = match.groups()
        events.append(
            {
                "edge": [source, target],
                "side": side,
                "nodes": node_ids(nodes),
                "result": None
                if result == "None"
                else {
                    "score_bits": f64_bits(score),
                    "dx_bits": f64_bits(dx),
                    "dy_bits": f64_bits(dy),
                },
            }
        )
    return events


def first_difference(left: Any, right: Any, path: str = "$") -> str | None:
    if type(left) is not type(right):
        return f"{path}: {left!r} != {right!r}"
    if isinstance(left, dict):
        for key in dict.fromkeys([*left, *right]):
            if key not in left or key not in right:
                return f"{path}.{key}: {left.get(key)!r} != {right.get(key)!r}"
            difference = first_difference(left[key], right[key], f"{path}.{key}")
            if difference:
                return difference
        return None
    if isinstance(left, list):
        for index, (left_item, right_item) in enumerate(zip(left, right)):
            difference = first_difference(left_item, right_item, f"{path}[{index}]")
            if difference:
                return difference
        if len(left) != len(right):
            return f"{path}.length: {len(left)} != {len(right)}"
        return None
    return None if left == right else f"{path}: {left!r} != {right!r}"


def first_event_difference(
    left: list[dict[str, Any]], right: list[dict[str, Any]]
) -> dict[str, Any] | None:
    for index, (go_event, rust_event) in enumerate(zip(left, right)):
        if go_event != rust_event:
            return {"index": index, "go": go_event, "rust": rust_event}
    if len(left) != len(right):
        return {
            "index": min(len(left), len(right)),
            "go": left[len(right)] if len(left) > len(right) else None,
            "rust": right[len(left)] if len(right) > len(left) else None,
        }
    return None


def run(binary: Path, seed: int, measured: bytes, trace_key: str | None = None) -> subprocess.CompletedProcess[bytes]:
    environment = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(("TALA_", "WEFTAN_")) and key not in ("DEV_MODE", "TEST_MODE")
    }
    if trace_key is not None:
        environment[trace_key] = "1"
    return subprocess.run(
        [str(binary), "layout", "--tala-seeds", str(seed)],
        input=measured,
        capture_output=True,
        timeout=180,
        env=environment,
        check=False,
    )


def record_for_case(fixture: str, seed: int, binaries: dict[str, Path]) -> dict[str, Any]:
    witness = ROOT / "target/parity-witnesses" / f"{fixture}.seed-{seed}.gap.trace"
    measured_path = witness / "measured.json"
    if not measured_path.is_file():
        raise FileNotFoundError(f"missing measured witness {measured_path}; capture it first")
    measured = measured_path.read_bytes()

    go = run(binaries["go_trace"], seed, measured, "TALA_TRACE_ALIGN")
    rust = run(binaries["rust_trace"], seed, measured, "WEFTAN_TRACE_ALIGN_TRIALS")
    go_release = run(binaries["go_release"], seed, measured)
    rust_release = run(binaries["rust_release"], seed, measured)
    for name, result in (("Go trace", go), ("Rust trace", rust), ("Go release", go_release), ("Rust release", rust_release)):
        if result.returncode != 0:
            raise RuntimeError(f"{fixture} seed {seed}: {name} exited {result.returncode}: {result.stderr[-2000:]!r}")

    go_events = parse_go(go.stderr)
    rust_events = parse_rust(rust.stderr)
    difference = first_difference(go_events, rust_events)
    event_difference = first_event_difference(go_events, rust_events)
    outputs_unchanged = go.stdout == go_release.stdout and rust.stdout == rust_release.stdout
    public_bytes_identical = go_release.stdout == rust_release.stdout
    return {
        "fixture": fixture,
        "seed": seed,
        "measured_sha256": digest(measured_path),
        "trial_count": len(go_events),
        "status": "matched" if difference is None and outputs_unchanged and public_bytes_identical else "divergent",
        "ordered_trial_trace_sha256": hashlib.sha256(json.dumps(go_events, separators=(",", ":")).encode()).hexdigest(),
        "go_stderr_sha256": hashlib.sha256(go.stderr).hexdigest(),
        "rust_stderr_sha256": hashlib.sha256(rust.stderr).hexdigest(),
        "diagnostics_leave_release_output_unchanged": outputs_unchanged,
        "public_bytes_identical": public_bytes_identical,
        **({"first_difference": difference} if difference else {}),
        **({"first_event_difference": event_difference} if event_difference else {}),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", action="append", choices=FIXTURES, dest="fixtures")
    parser.add_argument("--seed", action="append", type=int, dest="seeds")
    parser.add_argument("--output", type=Path, default=ROOT / "oracle-cache/reports/alignment-trial-parity-20260923.json")
    args = parser.parse_args()

    verify_source()
    go_release = REFERENCE / "bin/d2-oracle"
    go_trace = REFERENCE / "bin/d2-oracle-trace"
    verify_oracle(go_release)
    verify_oracle(go_trace, trace=True)
    binaries = {
        "go_release": go_release,
        "go_trace": go_trace,
        "rust_release": ROOT / "target/release/examples/tala-oracle",
        "rust_trace": ROOT / "target/diagnostic/examples/tala-oracle",
    }
    for binary in binaries.values():
        if not binary.is_file():
            raise FileNotFoundError(f"missing oracle binary: {binary}")

    results = [
        record_for_case(fixture, seed, binaries)
        for fixture in (args.fixtures or FIXTURES)
        for seed in (args.seeds or [1, 2])
    ]
    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-alignment-connected-node-trace-parity",
        "authority": "pristine D2 v0.9.0 with Go 1.27.0",
        "upstream_commit": MANIFEST["d2"]["commit"],
        "oracles": {
            name: {"sha256": digest(path)}
            for name, path in sorted(binaries.items())
        },
        "comparison": [
            "ordered edge and side trial identity",
            "ordered ConnectedNode identity list",
            "accepted/rejected result",
            "exact IEEE-754 score and selected translation bits",
            "diagnostic output equals each release output",
            "Go and Rust release public bytes",
        ],
        "matched": sum(result["status"] == "matched" for result in results),
        "total": len(results),
        "total_trials": sum(result["trial_count"] for result in results),
        "results": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"MATCH {report['matched']}/{report['total']} cases, {report['total_trials']} trials; wrote {args.output}")
    return int(report["matched"] != report["total"])


if __name__ == "__main__":
    raise SystemExit(main())
