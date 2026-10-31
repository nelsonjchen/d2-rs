#!/usr/bin/env python3
"""Shape d2lib-sequence-targets hook records into the d2-rs-wasm fixtures.

Recipe `d2lib-sequence-fixtures` (after `d2lib-sequence-targets`): pairs each
committed source (tools/d2js-capture/inputs/sequence-*.json and friends) with
the record the hook observed from the pinned d2lib.Compile (dagre, UTF-16).
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]


def expected_cases(cases, rows):
    return {"cases": [{"name": c["name"], "source": c["source"], "expected": r} for c, r in zip(cases, rows)]}


def successful_diagrams(cases, rows):
    # Only successful sources are committed; a fault or error here is drift.
    for c, r in zip(cases, rows):
        if r.get("panic") or r.get("error"):
            sys.exit(f"{c['name']}: expected a successful target, got {r.get('error') or r.get('panic_value')}")
    return {"cases": [{"source": c["source"], "diagram": r["diagram"]} for c, r in zip(cases, rows)]}


def single_target(cases, rows):
    (case,), (row,) = cases, rows
    if row.get("panic") or row.get("error"):
        sys.exit(f"{case['name']}: {row.get('error') or row.get('panic_value')}")
    return {"source_path": case["upstream_path"], "source": row["source"], "expected": row["diagram"]}


# (seed, hook record, fixture, scope, layout)
FAMILIES = [
    ("sequence-actor-label-policy.json", "tools/d2lib-sequence-targets/actor-label-policy.json",
     "crates/d2-rs-wasm/tests/fixtures/sequence-actor-label-policy-source-v090.json",
     "Sequence actor box/label/icon exports and raw lifeline routes: compile successes, compiler errors and source panics. "
     "Source-panic API semantics remain open and are never counted as matches.", expected_cases),
    ("sequence-target-expanded.json", "tools/d2lib-sequence-targets/target-expanded.json",
     "crates/d2-rs-wasm/tests/fixtures/sequence-target-export-expanded-source-v090.json",
     "Complete target exports and actual source errors; blank containers and intrinsic/sequence group/note/root cases.",
     expected_cases),
    ("sequence-grid-fault.json", "tools/d2lib-sequence-targets/grid-fault.json",
     "crates/d2-rs-wasm/tests/fixtures/sequence-grid-fault-source-v090.json",
     "Grid-actor sequence sources: complete successful targets and recovered nil-pointer faults; no exclusions.",
     expected_cases),
    ("sequence-outside-actor.json", "tools/d2lib-sequence-targets/outside-actor.json",
     "crates/d2-rs-wasm/tests/fixtures/sequence-outside-actor-source-v090.json",
     "Successful sequence targets with actors outside the lifeline area (person, label, icon positions); "
     "image icon-position source panics are separate pending contracts and not committed here.", successful_diagrams),
    ("target-projection-label.json", "tools/d2lib-sequence-targets/projection-label.json",
     "crates/d2-rs-wasm/tests/fixtures/target-projection-label-source-v090.json",
     "Complete actual original-Go target of the upstream flipt example, exercising grid extraction and remapped label identity.",
     single_target),
]


def main() -> None:
    commit = os.environ.get("D2RS_CAPTURE_COMMIT") or json.loads((ROOT / "reference-manifest.json").read_text())["d2"]["commit"]
    fixtures = Path(os.environ["D2RS_FIXTURES_DIR"])
    for seed, records, logical, scope, layout in FAMILIES:
        cases = json.loads((ROOT / "tools/d2js-capture/inputs" / seed).read_text())
        rows = json.loads((fixtures / records).read_text())
        if len(rows) != len(cases):
            sys.exit(f"{records}: {len(rows)} records for {len(cases)} sources")
        out = {"source_commit": commit, "scope": scope, **layout(cases, rows)}
        fixture_recipe.output(logical).write_text(json.dumps(out, indent=2) + "\n")


if __name__ == "__main__":
    main()
