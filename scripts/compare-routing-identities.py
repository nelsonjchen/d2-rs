#!/usr/bin/env python3
"""Compare ordered route-flavor identities and costs, not entire search traces."""
import argparse
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("witness", type=Path)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
go = json.loads((args.witness / "go.routing.json").read_text())
rust = json.loads((args.witness / "rust.routing.json").read_text())
rows = []
for i in range(max(len(go), len(rust))):
    if i >= len(go) or i >= len(rust):
        rows.append({"event": i, "matched": False, "missing_event": True})
        continue
    g, r = go[i], rust[i]
    identities = lambda event: [(route["id"], route["from"], route["to"]) for route in event["routes"]]
    row = {"event": i, "go_flavor": g["flavor"], "rust_flavor": r["flavor"],
           "ordered_identities_match": identities(g) == identities(r),
           "distance_bits_match": g["distance_bits"] == r["distance_bits"],
           "go_route_count": len(g["routes"]), "rust_route_count": len(r["routes"])}
    row["matched"] = row["ordered_identities_match"] and row["distance_bits_match"] and g["flavor"] == r["flavor"]
    if not row["ordered_identities_match"]:
        row["go_identities"] = identities(g)
        row["rust_identities"] = identities(r)
    rows.append(row)
report = {"scope": "ordered source edge/from/to identities, flavor and exact distance bits only",
          "capture": json.loads((args.witness / "manifest.json").read_text()),
          "matched": sum(r["matched"] for r in rows), "remaining": sum(not r["matched"] for r in rows),
          "full_search_trace_parity": False, "results": rows}
args.output.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({k: report[k] for k in ("scope", "matched", "remaining", "full_search_trace_parity")}, indent=2))
raise SystemExit(not rows or bool(report["remaining"]))
