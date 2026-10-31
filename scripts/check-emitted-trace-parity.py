#!/usr/bin/env python3
"""Compare complete emitted TALA traces for one fixture and selected seeds.

The checker deliberately compares the captured event families without
normalizing operation values or order.  It is useful for replaying a witness
after a source-shaped fix and for replacing stale, one-off qualification
reports.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

from references import ROOT, verify_source

FAMILIES = ("stages", "placement", "candidates", "scores", "packing", "routing", "rng", "gap")


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def comparable_event(event: dict[str, Any], family: str) -> dict[str, Any]:
    """Normalize snapshot record serialization without weakening event order."""
    value = dict(event)
    if family == "stages":
        value.pop("seed", None)
    if family in {"stages", "placement"}:
        for collection in ("nodes", "edges"):
            records = value.get(collection)
            if isinstance(records, list) and all(
                isinstance(record, dict) and isinstance(record.get("id"), str)
                for record in records
            ):
                value[collection] = [
                    {
                        **{key: record[key] for key in ("stable_id", "id") if key in record},
                        **{
                            key: item
                            for key, item in record.items()
                            if key not in {"stable_id", "id"}
                        },
                    }
                    for record in sorted(
                        records,
                        key=lambda record: str(record.get("stable_id", record["id"])),
                    )
                ]
    return value


def load(path: Path, family: str) -> list[dict[str, Any]]:
    events = json.loads(path.read_text())
    return [comparable_event(event, family) for event in events]


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


def difference_profile(left: list[dict[str, Any]], right: list[dict[str, Any]]) -> dict[str, Any]:
    """Summarize strict event differences without changing comparison semantics."""
    changed_fields: dict[str, int] = {}
    different_events = 0
    ordered_connected_only_events = 0
    ordered_connected_same_members = 0
    for index in range(max(len(left), len(right))):
        left_event = left[index] if index < len(left) else None
        right_event = right[index] if index < len(right) else None
        if left_event == right_event:
            continue
        different_events += 1
        if not isinstance(left_event, dict) or not isinstance(right_event, dict):
            changed_fields["$event"] = changed_fields.get("$event", 0) + 1
            continue
        differing_keys = [
            key
            for key in dict.fromkeys([*left_event, *right_event])
            if key not in left_event
            or key not in right_event
            or left_event[key] != right_event[key]
        ]
        for key in differing_keys:
            changed_fields[key] = changed_fields.get(key, 0) + 1
        if differing_keys == ["ordered_connected"]:
            ordered_connected_only_events += 1
            left_order = left_event["ordered_connected"]
            right_order = right_event["ordered_connected"]
            if isinstance(left_order, list) and isinstance(right_order, list):
                if sorted(left_order) == sorted(right_order):
                    ordered_connected_same_members += 1
    return {
        "different_event_count": different_events,
        "event_count_delta": len(left) - len(right),
        "changed_top_level_fields": dict(sorted(changed_fields.items())),
        "ordered_connected_only_event_count": ordered_connected_only_events,
        "ordered_connected_same_members_event_count": ordered_connected_same_members,
    }


def classify_event_family(left: list[dict[str, Any]], right: list[dict[str, Any]]) -> tuple[str, str | None]:
    difference = first_difference(left, right)
    if difference is not None:
        return "divergent", difference
    if not left:
        return "unexercised", None
    return "matched", None


def capture(fixture: str, fixtures_dir: Path, seed: int, connected_order: str | None) -> None:
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
    if connected_order:
        command.extend(["--connected-order", connected_order])
    subprocess.run(command, cwd=ROOT, check=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture")
    parser.add_argument("--fixtures-dir", type=Path, default=ROOT / "tests/d2")
    parser.add_argument("--seed", action="append", type=int, required=True, dest="seeds")
    parser.add_argument("--capture", action="store_true", help="recapture both oracles before comparing")
    parser.add_argument("--connected-order", metavar="NODE_ID", help="also capture connected BFS and map iteration order; use 'all' or a TALA node ID")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    source = verify_source()
    fixture_path = args.fixtures_dir / args.fixture
    if not fixture_path.is_file():
        raise SystemExit(f"fixture not found: {fixture_path}")
    if args.capture:
        for seed in args.seeds:
            capture(args.fixture, args.fixtures_dir, seed, args.connected_order)

    stem = fixture_path.stem
    results: list[dict[str, Any]] = []
    for seed in args.seeds:
        witness = ROOT / "target/parity-witnesses" / f"{stem}.seed-{seed}.gap.trace"
        if not witness.is_dir():
            raise SystemExit(f"missing witness: {witness}; pass --capture")
        manifest = json.loads((witness / "manifest.json").read_text())
        families_to_compare = list(FAMILIES)
        if manifest.get("capture_options", {}).get("connected_order"):
            families_to_compare.append("connected")
        families: dict[str, dict[str, Any]] = {}
        for family in families_to_compare:
            go_events = load(witness / f"go.{family}.json", family)
            rust_events = load(witness / f"rust.{family}.json", family)
            family_status, difference = classify_event_family(go_events, rust_events)
            families[family] = {
                "status": family_status,
                "go_events": len(go_events),
                "rust_events": len(rust_events),
                "difference_profile": difference_profile(go_events, rust_events),
                **({"first_difference": difference} if difference else {}),
            }
        public = (witness / "go.stdout.json").read_bytes() == (witness / "rust.stdout.json").read_bytes()
        results.append(
            {
                "fixture": args.fixture,
                "seed": seed,
                "status": (
                    "matched"
                    if public and all(item["status"] == "matched" for item in families.values())
                    else "unexercised"
                    if public and all(item["status"] in {"matched", "unexercised"} for item in families.values())
                    else "divergent"
                ),
                "public_bytes_identical": public,
                "event_families": families,
                "witness": str(witness),
                "go_trace_sha256": manifest["oracles"]["go"]["sha256"],
                "rust_trace_sha256": manifest["oracles"]["rust"]["sha256"],
                "go_release_sha256": manifest["oracles"]["go"]["base_sha256"],
                "rust_release_sha256": manifest["oracles"]["rust"]["base_sha256"],
            }
        )

    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-tala-emitted-trace-parity",
        "authority": "pristine D2 v0.9.0 with Go 1.27.0",
        "upstream_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "fixture": args.fixture,
        "fixture_sha256": digest(fixture_path),
        "seeds": args.seeds,
        "event_scope": sorted({family for result in results for family in result["event_families"]}),
        "normalization": [
            "remove Rust-only stage.seed",
            "sort stable-ID node and edge records within stage/placement snapshots",
            "retain event, candidate, connected-traversal, and all other payload order",
            "when captured, represent connected traversal IDs as integers and retain each actual cluster/container iteration order",
        ],
        "matched": sum(result["status"] == "matched" for result in results),
        "total": len(results),
        "results": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"MATCH {report['matched']}/{report['total']}; wrote {args.output}")
    return int(report["matched"] != report["total"])


if __name__ == "__main__":
    raise SystemExit(main())
