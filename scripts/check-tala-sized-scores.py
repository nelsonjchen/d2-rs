#!/usr/bin/env -S uv run python
# SPDX-License-Identifier: MPL-2.0
"""Compare ordered sized candidate scores and selections, preserving f64 bits.

This checks the existing diagnostic SIZED_SCORE_GO/RUST records only. It does
not establish candidate eligibility, RNG, traversal, or full execution parity.
Decimal numbers in the raw records are round-trip f64 representations. Node
IDs, event order, signed zero, infinities, score, and prior best are retained.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import struct


SCORE = re.compile(r"^SIZED_SCORE_(?:GO|RUST) phase=(edge|total) node=(-?\d+) point=([^,]+),([^ ]+) score=([^ ]+) least=([^ ]+)$")
SELECTED = re.compile(r"^SIZED_SCORE_(?:GO|RUST) selected node=(-?\d+) point=([^,]+),([^ ]+) moved=(true|false)$")


def bits(value: str) -> str:
    number = float(value)
    if math.isnan(number):
        raise ValueError("textual NaN does not preserve its IEEE-754 payload")
    return struct.pack(">d", number).hex()


def records(path: Path) -> list[dict]:
    result = []
    for line in path.read_text().splitlines():
        if match := SCORE.fullmatch(line):
            phase, node, x, y, score, least = match.groups()
            result.append({"phase": phase, "node": int(node),
                           "point_bits": [bits(x), bits(y)],
                           "score_bits": bits(score), "least_bits": bits(least)})
        elif match := SELECTED.fullmatch(line):
            node, x, y, moved = match.groups()
            result.append({"phase": "selected", "node": int(node),
                           "point_bits": [bits(x), bits(y)], "moved": moved == "true"})
        elif line.startswith(("SIZED_SCORE_GO ", "SIZED_SCORE_RUST ")):
            raise ValueError(f"unrecognized score record: {line}")
    if not result or not any(record["phase"] == "selected" for record in result):
        raise ValueError(f"missing candidate score or selection records in {path}")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go", type=Path, required=True)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    events = {side: records(getattr(args, side)) for side in ("go", "rust")}
    common = min(map(len, events.values()))
    mismatch = next((i for i in range(common) if events["go"][i] != events["rust"][i]), None)
    passed = events["go"] == events["rust"]
    report = {"passed": passed, "scope": __doc__, "full_trace_parity": False,
              "captures": {side: {"path": str(getattr(args, side)),
                                   "sha256": hashlib.sha256(getattr(args, side).read_bytes()).hexdigest()}
                           for side in events},
              "counts": {side: len(rows) for side, rows in events.items()},
              "first_difference": None if mismatch is None else {
                  "index": mismatch, "go": events["go"][mismatch], "rust": events["rust"][mismatch]},
              "unmatched_tails": {side: rows[common:] for side, rows in events.items()},
              "records": events}
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("passed", "counts", "first_difference")}))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
