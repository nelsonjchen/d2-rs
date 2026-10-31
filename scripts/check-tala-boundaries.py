#!/usr/bin/env python3
"""Compare ordered, exact TALA stage or placement boundary captures.

This is a boundary gate, not an operation-level trace comparator. Geometry is
already recorded as IEEE-754 bit strings by both observers. No coordinates,
identities, node/edge ordering or missing tails are normalized. The Rust stage
observer includes seed metadata that Go's observer omits; validate any recorded
seed against the explicit single-seed invocation before removing that field.
"""

import argparse
import hashlib
import json
from pathlib import Path


def read_events(path, event, seed):
    selected = []
    for line in path.read_text().splitlines():
        if not line.startswith("{"):
            continue
        row = json.loads(line)
        if row.get("event") != event:
            continue
        if "seed" in row:
            assert row["seed"] == seed, (path, row["seed"], seed)
            row.pop("seed")
        selected.append(row)
    assert selected, f"Missing {event} events: {path}"
    return selected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go", type=Path, required=True)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--event", choices=("stage", "placement"), required=True)
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    rows = {side: read_events(getattr(args, side), args.event, args.seed)
            for side in ("go", "rust")}
    common = min(map(len, rows.values()))
    differences = [{"index": index, "go": rows["go"][index],
                    "rust": rows["rust"][index]}
                   for index in range(common)
                   if rows["go"][index] != rows["rust"][index]]
    result = {
        "event": args.event,
        "explicit_invocation_seed": args.seed,
        "captures": {side: {"path": str(getattr(args, side)),
                             "sha256": hashlib.sha256(getattr(args, side).read_bytes()).hexdigest()}
                     for side in rows},
        "counts": {side: len(events) for side, events in rows.items()},
        "boundaries_equal": rows["go"] == rows["rust"],
        "difference_count": len(differences),
        "differences": differences,
        "unmatched_tails": {side: events[common:] for side, events in rows.items()},
        "normalization": "Recorded seed metadata is validated then omitted; nothing else.",
        "full_operation_trace_parity": False,
    }
    args.report.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: result[key] for key in
                      ("event", "counts", "boundaries_equal", "difference_count")}))
    raise SystemExit(0 if result["boundaries_equal"] else 1)


if __name__ == "__main__":
    main()
