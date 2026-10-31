#!/usr/bin/env python3
"""elk-go provider fixture generator (tools/fixtures/langlibelk-elk-providers.toml).

usage: check-elk-providers.py FAMILY

Builds tools/elk-oracle against the pinned elk-go module, runs every case of
FAMILY (scripts/elk_cases) through elk.LayoutJSON in its own process with a
3-second deadline (native time; scaled under arm64 emulation, see
scripts/elk_oracle.py), and writes rows {name, input, source_exit, source_stderr,
expected} to the staged recipe output. Errors, Go runtime crashes and
deadline expiries are recorded as such (expected = null), never as passing
geometry. Go runtime crash dumps keep only their stable header
(elk_oracle.stable_stderr), so reruns are byte-identical.
`cargo test -p d2-rs-elk --features oracle-fixtures` compares.
"""
import json
import subprocess
import sys

from elk_cases import mrtree, radial, spore, stress
from elk_oracle import run, stable_stderr, wire_oracle, write_json
from fixture_recipe import output

FIXTURES = "crates/d2-rs-elk/src/fixtures/"
FAMILIES = {
    "mrtree": (mrtree.base, "original_mrtree.json"),
    "mrtree-expanded": (mrtree.expanded, "original_mrtree_expanded.json"),
    "radial": (radial.base, "original_radial.json"),
    "radial-expanded": (radial.expanded, "original_radial_expanded.json"),
    "radial-fractional": (radial.fractional, "original_radial_fractional.json"),
    "spore": (spore.base, "original_spore.json"),
    "spore-expanded": (spore.expanded, "original_spore_expanded.json"),
    "spore-root-transfer": (spore.root_transfer, "original_spore_root_transfer.json"),
    "force-stress": (stress.base, "original_force_stress.json"),
    "force-stress-expanded": (stress.followup, "original_force_stress_expanded.json"),
}
DEADLINE = 3
TIMEOUT_NOTE = f"Source exceeded explicit {DEADLINE}-second per-case deadline"

family = sys.argv[1]
cases, name = FAMILIES[family]
oracle = wire_oracle()
rows = []
for row in cases():
    try:
        q = run(oracle, json.dumps(row["input"]).encode(), timeout=DEADLINE)
        stderr = stable_stderr(q.returncode, q.stderr.decode())
        rows.append(dict(**row, source_exit=q.returncode, source_stderr=stderr,
                         expected=json.loads(q.stdout) if q.returncode == 0 else None))
    except subprocess.TimeoutExpired:
        rows.append(dict(**row, source_exit=124, source_stderr=TIMEOUT_NOTE, expected=None))
    if rows[-1]["source_exit"]:
        print("source failure", row["name"], rows[-1]["source_exit"], rows[-1]["source_stderr"][:200], flush=True)
if family == "force-stress-expanded":
    fixture = dict(
        source_successes=[r for r in rows if r["source_exit"] == 0],
        unqualified_source_timeouts=[r for r in rows if r["source_exit"] != 0],
        note="The two child-Random controls explicitly seed the child. The original clock-seeded outputs remain "
             "in source-attempt1.json; a clock-seeded run does not define one exact expected geometry.")
else:
    fixture = rows
write_json(output(FIXTURES + name), fixture)
print(f"{family}: {sum(r['source_exit'] == 0 for r in rows)}/{len(rows)} source successes")
