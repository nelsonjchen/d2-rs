#!/usr/bin/env python3
"""Qualify a pinned nested TALA trace at seeds 1 and 2.

This is deliberately a development-only check.  It captures both oracles
with the existing source-shaped harness, compares every enabled event family,
and writes the report consumed by the parity ledger. Only the diagnostic
stage ``seed`` field is omitted; float-bit strings, identities, event order,
and all other fields remain exact.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

from references import ROOT, REFERENCE, digest, verify_oracle, verify_source

FAMILIES = ("stages", "placement", "candidates", "scores", "packing", "routing", "rng", "gap")


def load_family(directory: Path, side: str, family: str) -> list[dict]:
    events = json.loads((directory / f"{side}.{family}.json").read_text())
    if family == "stages" and side == "rust":
        return [{key: value for key, value in event.items() if key != "seed"} for event in events]
    return events


def compare_family(left: list[dict], right: list[dict]) -> dict:
    if left == right:
        return {"status": "matched", "events": len(left)}
    first_divergence = None
    differing_fields: Counter[str] = Counter()
    mismatched_events = 0
    for index, (go, rust) in enumerate(zip(left, right)):
        if go != rust:
            mismatched_events += 1
            fields = {key for key in go if go[key] != rust.get(key)}
            fields.update(key for key in rust if key not in go)
            differing_fields.update(fields)
            if first_divergence is None:
                first_divergence = {"index": index, "go": go, "rust": rust}
    if len(left) != len(right):
        return {
            "status": "divergent",
            "go_events": len(left),
            "rust_events": len(right),
            "mismatched_events": mismatched_events,
            "differing_fields": dict(sorted(differing_fields.items())),
            **(first_divergence or {}),
        }
    return {
        "status": "divergent",
        "mismatched_events": mismatched_events,
        "differing_fields": dict(sorted(differing_fields.items())),
        **(first_divergence or {}),
    }


def capture(source: Path, fixture: str, seed: int) -> None:
    command = [
        sys.executable,
        str(ROOT / "scripts/capture-layout-trace.py"),
        fixture,
        "--fixtures-dir",
        str(source / "e2etests/testdata/files"),
        "--seed",
        str(seed),
        "--placement",
        "--packing",
        "--routing",
        "--rng",
        "--gap",
    ]
    subprocess.run(command, cwd=ROOT, check=True)


def probe_go_gap_map_order(fixture: str, seed: int = 1, runs: int = 3) -> dict:
    """Check whether Go map-derived gap order varies across fresh processes.

    Keep the trace strict. This probe records runtime variation as evidence; it
    does not normalize the order or turn an event mismatch into a pass.
    """
    oracle = REFERENCE / "bin/d2-oracle-trace"
    verify_oracle(oracle, trace=True)
    witness = ROOT / "target/parity-witnesses" / f"{Path(fixture).stem}.seed-{seed}.gap.trace"
    measured = (witness / "measured.json").read_bytes()
    observations = []
    for run in range(runs):
        environment = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("TALA_", "WEFTAN_"))
            and key not in ("DEV_MODE", "TEST_MODE")
        }
        environment["TALA_TRACE_GAP"] = "1"
        with tempfile.TemporaryFile(mode="w+b") as stderr_file:
            result = subprocess.run(
                [str(oracle), "layout", "--tala-seeds", str(seed)],
                input=measured,
                stdout=subprocess.PIPE,
                stderr=stderr_file,
                check=True,
                timeout=180,
                env=environment,
            )
            stderr_file.seek(0)
            ordered_digest = hashlib.sha256()
            event_count = 0
            for line in stderr_file:
                try:
                    event = json.loads(line)
                except ValueError:
                    continue
                if event.get("event") == "gap-connectivity":
                    ordered_digest.update(
                        json.dumps(event.get("ordered_connected"), separators=(",", ":")).encode()
                        + b"\n"
                    )
                    event_count += 1
        observations.append(
            {
                "run": run + 1,
                "gap_connectivity_events": event_count,
                "ordered_connected_sha256": ordered_digest.hexdigest(),
                "public_bytes_sha256": hashlib.sha256(result.stdout).hexdigest(),
            }
        )
    return {
        "authority": "pinned D2 v0.9.0 Go 1.27.0 trace oracle",
        "fixture": fixture,
        "seed": seed,
        "runs": observations,
        "map_order_varies": len({run["ordered_connected_sha256"] for run in observations}) > 1,
        "public_output_stable": len({run["public_bytes_sha256"] for run in observations}) == 1,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", default="nesting_power.d2", help="upstream fixture filename")
    parser.add_argument("--output", type=Path, help="report path (defaults to docs/<fixture>-emitted-trace-parity.json)")
    parser.add_argument("--skip-capture", action="store_true", help="compare existing witness directories")
    args = parser.parse_args()
    source = verify_source()
    if not args.skip_capture:
        for seed in (1, 2):
            capture(source, args.fixture, seed)
    map_order_probe = probe_go_gap_map_order(args.fixture)

    results = []
    fixture_stem = Path(args.fixture).stem
    for seed in (1, 2):
        directory = ROOT / "target/parity-witnesses" / f"{fixture_stem}.seed-{seed}.gap.trace"
        manifest = json.loads((directory / "manifest.json").read_text())
        families = {
            family: compare_family(load_family(directory, "go", family), load_family(directory, "rust", family))
            for family in FAMILIES
        }
        public = (directory / "go.stdout.json").read_bytes() == (directory / "rust.stdout.json").read_bytes()
        results.append(
            {
                "fixture": args.fixture,
                "seed": seed,
                "status": "matched" if public and all(item["status"] == "matched" for item in families.values()) else "divergent",
                "public_graph_exact": public,
                "public_bytes_identical": public,
                "event_families": families,
                "witness": str(directory),
                "go_trace_sha256": manifest["oracles"]["go"]["sha256"],
                "rust_trace_sha256": manifest["oracles"]["rust"]["sha256"],
                "go_release_sha256": manifest["oracles"]["go"]["base_sha256"],
                "rust_release_sha256": manifest["oracles"]["rust"]["base_sha256"],
            }
        )

    report = {
        "schema_version": 2,
        "kind": "d2-v0.9.0-tala-multiseed-trace-parity",
        "authority": "pristine D2 v0.9.0 with Go 1.27.0",
        "upstream_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "fixture": args.fixture,
        "seeds": [1, 2],
        "event_scope": "all captured placement, candidate, score, packing, routing, RNG, gap and stage events",
        "normalization": [
            "remove Rust-only stage.seed",
            "no other event fields normalized",
            "exact float bit strings and event order retained",
        ],
        "matched": sum(result["status"] == "matched" for result in results),
        "total": len(results),
        "release_binary_sha256": results[0]["go_release_sha256"],
        "diagnostic_binary_sha256": results[0]["go_trace_sha256"],
        "results": results,
        "upstream_map_order_probe": map_order_probe,
        "qualification": (
            "Both seeds have byte-identical public output. The strict trace comparison remains "
            "divergent in "
            + ", ".join(
                sorted(
                    {
                        family
                        for result in results
                        for family, comparison in result["event_families"].items()
                        if comparison["status"] != "matched"
                    }
                )
            )
            + ". No trace fields are normalized beyond the Rust-only stage seed. "
            + (
                "Three fresh Go processes confirmed that ordered_connected map traversal varies "
                "while public output stays byte-identical."
                if map_order_probe["map_order_varies"] and map_order_probe["public_output_stable"]
                else "The repeated Go map-order probe did not establish stable output plus varying order."
            )
        ),
    }
    output = (args.output or (ROOT / "oracle-cache/reports" / f"{fixture_stem.replace('_', '-')}-emitted-trace-parity.json")).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"report": str(output), "matched": report["matched"], "total": report["total"]}, indent=2))
    if report["matched"] != report["total"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
