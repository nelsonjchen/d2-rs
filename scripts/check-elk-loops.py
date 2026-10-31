#!/usr/bin/env python3
"""Compare self-loop wire geometry without coordinate rounding or translation."""
import hashlib
import argparse
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
go = ROOT / ".reference/bin/elk-wire-oracle"
rust = ROOT / "target/debug/examples/elk-wire"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--labels", action="store_true", help="attach unequal measured labels to every loop")
parser.add_argument("--nested", action="store_true", help="place the loop node inside a compound node")
parser.add_argument("--multiple-labels", action="store_true", help="attach two unequal labels to each loop (implies --labels)")
parser.add_argument("--nested-label-spacing", action="store_true", help="set label-label spacing on the nested graph")
args = parser.parse_args()
args.labels = args.labels or args.multiple_labels

def nodes_recursive(nodes):
    for node in nodes:
        yield [node["id"], node.get("x", 0), node.get("y", 0), node["width"], node["height"]]
        yield from nodes_recursive(node.get("children", []))

def run(binary, graph):
    result = subprocess.run([str(binary)], input=json.dumps(graph), text=True,
                            capture_output=True, timeout=30)
    if result.returncode:
        return {"error": result.stderr, "exit": result.returncode}
    value = json.loads(result.stdout)
    # Missing root dimensions remain visible failures, not implicit zeros.
    return {
        "root": [value.get("width"), value.get("height")],
        "nodes": list(nodes_recursive(value.get("children", []))),
        "edges": [[e["id"], [[s["startPoint"], s.get("bendPoints", []), s["endPoint"]]
                              for s in e.get("sections", [])]] for e in value["edges"]],
        "labels": [[e["id"], [[label.get("text", ""), label.get("x", 0), label.get("y", 0),
                                label.get("width", 0), label.get("height", 0)]
                               for label in e.get("labels", [])]] for e in value["edges"]],
    }

cases = []

def first_difference(expected, actual, path="$"):
    """Retain a concrete first mismatch rather than only a failed case ID."""
    if isinstance(expected, dict) and isinstance(actual, dict):
        if expected.keys() != actual.keys():
            return {"path": path, "expected_keys": list(expected), "actual_keys": list(actual)}
        for key in expected:
            difference = first_difference(expected[key], actual[key], f"{path}.{key}")
            if difference:
                return difference
    elif isinstance(expected, list) and isinstance(actual, list):
        if len(expected) != len(actual):
            return {"path": path, "expected_length": len(expected), "actual_length": len(actual)}
        for index, (left, right) in enumerate(zip(expected, actual)):
            difference = first_difference(left, right, f"{path}[{index}]")
            if difference:
                return difference
    elif expected != actual:
        return {"path": path, "expected": expected, "actual": actual}
    return None

for mode in ["NORTH", "NORTH_SOUTH", "EQUALLY"]:
    for direction in ["RIGHT", "DOWN", "UP", "LEFT"]:
        for count in range(1, 9):
            graph = {
                "id": "root",
                "layoutOptions": {"elk.algorithm": "layered", "elk.direction": direction,
                                  "elk.padding": "[top=0,left=0,bottom=0,right=0]", "elk.spacing.nodeSelfLoop": 50},
                "children": [{"id": "A", "x": 0, "y": 0, "width": 40, "height": 30,
                              "layoutOptions": {"elk.layered.edgeRouting.selfLoopDistribution": mode}}],
                "edges": [{"id": f"e{i}", "sources": ["A"], "targets": ["A"], "container": "root"}
                          for i in range(count)],
            }
            if args.labels:
                if args.multiple_labels:
                    graph["layoutOptions"]["elk.spacing.labelLabel"] = 4
                for index, edge in enumerate(graph["edges"]):
                    edge["labels"] = [{"text": f"loop {index}", "x": 0, "y": 0,
                                       "width": 20 + 7 * index, "height": 10 + 3 * index}]
                    if args.multiple_labels:
                        edge["labels"].append({"text": f"detail {index}", "x": 0, "y": 0,
                                               "width": 13 + 5 * index, "height": 7 + 2 * index})
            if args.nested:
                graph["children"] = [{"id": "parent", "x": 0, "y": 0, "width": 0, "height": 0,
                    "layoutOptions": {"elk.algorithm": "layered", "elk.direction": direction},
                    "children": graph["children"]}]
                if args.nested_label_spacing:
                    graph["children"][0]["layoutOptions"]["elk.spacing.labelLabel"] = 4
                graph["layoutOptions"]["elk.hierarchyHandling"] = "INCLUDE_CHILDREN"
                for edge in graph["edges"]:
                    edge["container"] = "parent"
            expected, actual = run(go, graph), run(rust, graph)
            passed = expected == actual and "error" not in expected
            content_matches = ("error" not in expected and "error" not in actual
                and {k: v for k, v in expected.items() if k != "root"}
                == {k: v for k, v in actual.items() if k != "root"})
            case = {"id": f"{mode}/{direction}/{count}", "passed": passed,
                    "content_geometry_matches": content_matches}
            if not passed:
                case.update(input=graph, expected=expected, actual=actual,
                            first_difference=first_difference(expected, actual))
            cases.append(case)
report = {
    "scope": "Loop root dimensions, recursive node boxes, root-listed sections and label geometry; no tolerance, zero root padding; excludes traces and section identity",
    "labelled": args.labels,
    "nested": args.nested,
    "multiple_labels": args.multiple_labels,
    "nested_label_spacing": args.nested_label_spacing,
    "content_geometry_passed": sum(c["content_geometry_matches"] for c in cases),
    "binary_sha256": {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in [go, rust]},
    "passed": sum(c["passed"] for c in cases), "total": len(cases), "cases": cases,
}
path = ROOT / ("oracle-cache/reports/elk-labelled-loop-wire-parity.json" if args.labels else "oracle-cache/reports/elk-loop-wire-parity.json")
if args.nested:
    path = path.with_name(path.stem + "-nested.json")
if args.multiple_labels:
    path = path.with_name(path.stem + "-multiple.json")
if args.nested_label_spacing:
    path = path.with_name(path.stem + "-child-spacing.json")
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(report, indent=2) + "\n")
print(f'{report["passed"]}/{report["total"]} exact geometry matches; {path}')
raise SystemExit(0 if report["passed"] == report["total"] else 1)
