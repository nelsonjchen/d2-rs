#!/usr/bin/env python3
"""Localize stage snapshot differences without claiming execution-trace parity.

Every original event index and unmatched stage is retained in the report.
Matching a stage name/occurrence is a debugging aid, not proof the calls align.
No coordinates, identities, array orders or floating-point bits are normalized.
"""
import argparse
from collections import Counter
import importlib.util
import json
from pathlib import Path
from references import ROOT

spec = importlib.util.spec_from_file_location("boundaries", ROOT / "scripts/check-boundaries.py")
boundaries = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boundaries)


def indexed(events):
    counts = Counter()
    result = {}
    for index, event in enumerate(events):
        name = event["stage"]
        occurrence = counts[name]
        counts[name] += 1
        result[(name, occurrence)] = (index, event)
    return result


def edge_geometry(event):
    # Independent field comparison, not an ID normalization: the complete
    # edge comparison still reports identity differences alongside this view.
    return [{key: edge[key] for key in ("index", "from", "to", "route") if key in edge}
            for edge in event.get("edges", [])]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    directory = ROOT / "target/parity-witnesses" / f"{Path(args.fixture).stem}.seed-{args.seed}.trace"
    manifest = json.loads((directory / "manifest.json").read_text())
    go = indexed(json.loads((directory / "go.stages.json").read_text()))
    rust = indexed(json.loads((directory / "rust.stages.json").read_text()))
    rows = []
    for key in list(go) + [key for key in rust if key not in go]:
        row = {"stage": key[0], "occurrence": key[1]}
        if key not in go or key not in rust:
            side = "go" if key in go else "rust"
            row.update({"unmatched": side, "event_index": (go if side == "go" else rust)[key][0]})
        else:
            gi, g = go[key]
            ri, r = rust[key]
            row.update({"go_event_index": gi, "rust_event_index": ri,
                        "nodes_difference": boundaries.difference(g.get("nodes"), r.get("nodes"), "$.nodes"),
                        "edge_geometry_difference": boundaries.difference(edge_geometry(g), edge_geometry(r), "$.edges"),
                        "edges_difference": boundaries.difference(g.get("edges"), r.get("edges"), "$.edges")})
        rows.append(row)
    packing = None
    if ("BinPack", 0) in go and ("BinPack", 0) in rust:
        gi, ge = go[("BinPack", 0)]
        ri, re = rust[("BinPack", 0)]
        gprev = next((event for index, event in go.values() if index == gi - 1), None)
        rprev = next((event for index, event in rust.values() if index == ri - 1), None)
        if gprev is not None and rprev is not None:
            packing = {"go_before_index": gi - 1, "rust_before_index": ri - 1,
                       "go_before_stage": gprev["stage"], "rust_before_stage": rprev["stage"],
                       "before_nodes_difference": boundaries.difference(gprev.get("nodes"), rprev.get("nodes")),
                       "after_nodes_difference": boundaries.difference(ge.get("nodes"), re.get("nodes"))}
    report = {"scope": "name/occurrence snapshot localization only; unmatched events explicitly retained",
              "capture": manifest, "comparisons": rows, "first_binpack_boundary": packing}
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    for row in rows:
        if row.get("nodes_difference") or row.get("edge_geometry_difference") or row.get("unmatched"):
            print(json.dumps(row))


if __name__ == "__main__":
    main()
