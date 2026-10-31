"""Compare ordered generation/publication checkpoints without trimming tails."""
import argparse
import hashlib
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go", type=Path, required=True)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--nodes", type=int, required=True)
    parser.add_argument("--edges", type=int, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--include-heap", action="store_true",
                        help="Also compare every captured heap event, including edge order")
    args = parser.parse_args()
    event_names = {"generation-work", "publication-work"}
    if args.include_heap:
        event_names.add("route-search-heap")
    captures = {}
    rows = {}
    for side in ("go", "rust"):
        path = getattr(args, side)
        captures[side] = {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        rows[side] = [event for line in path.read_text().splitlines()
                      if line.startswith("{")
                      for event in [json.loads(line)]
                      if event.get("scope_nodes") == args.nodes
                      and event.get("scope_edges") == args.edges
                      and event.get("event") in event_names]
    assert all(rows.values()), "Both captures must contain checkpoints"
    if args.include_heap:
        assert all(any(row["event"] == "route-search-heap" for row in events)
                   for events in rows.values()), "Both captures must contain heap events"
    groups = sorted({(row["flavor"], row["event"]) for events in rows.values() for row in events})
    comparisons = []
    for flavor, event in groups:
        selected = {side: [row for row in events if row["flavor"] == flavor and row["event"] == event]
                    for side, events in rows.items()}
        common = min(map(len, selected.values()))
        mismatch = next((i for i in range(common) if selected["go"][i] != selected["rust"][i]), None)
        comparisons.append({"flavor": flavor, "event": event,
                            "counts": {side: len(events) for side, events in selected.items()},
                            "equal": selected["go"] == selected["rust"],
                            "first_difference": None if mismatch is None else {
                                "index": mismatch, "go": selected["go"][mismatch], "rust": selected["rust"][mismatch]},
                            "unmatched_tails": {side: events[common:] for side, events in selected.items()}})
    passed = all(item["equal"] for item in comparisons)
    args.report.write_text(json.dumps({"captures": captures, "scope_nodes": args.nodes,
                                      "scope_edges": args.edges, "checkpoints_equal": passed,
                                      "full_trace_parity": False, "comparisons": comparisons}, indent=2) + "\n")
    print(json.dumps({"checkpoints_equal": passed, "comparisons": comparisons}, indent=2))
    raise SystemExit(0 if passed else 1)


if __name__ == "__main__":
    main()
