#!/usr/bin/env python3
"""Seeded Dagre/ELK feature-combination hunt, with complete output inventories.

Reuses graph-differential's capture, map-seed resampling and minimization.
Generated inputs and oracle outputs are temporary; only findings are retained.
"""
import argparse
import concurrent.futures
import importlib.util
import json
from pathlib import Path
import random
import tempfile

from fuzz_common import first_error

_spec = importlib.util.spec_from_file_location(
    "graph_differential", Path(__file__).with_name("graph-differential.py"))
graph = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(graph)

FAMILIES = ("containers", "grids", "sequences", "ports", "short-shapes")


def compare(case, rust, go, minimize=True):
    index, source, flags = case
    results = []
    for layout in ("dagre", "elk"):
        with tempfile.TemporaryDirectory(prefix="layout-hunt-") as tmp:
            root = Path(tmp)
            gc, ge, gh = graph.outcome(go, source, layout, flags, root, "go")
            rc, re, rh = graph.outcome(rust, source, layout, flags, root, "rust")
            if "panic:" in ge or isinstance(gc, int) and gc < 0:
                kind = "go-crash"
            elif gc == "timeout" or rc == "timeout":
                kind = "timeout"
            elif gc == "invalid-output" or rc == "invalid-output":
                kind = "invalid-output"
            elif gc == rc == 0 and gh == rh:
                kind = "rendered-match"
            elif gc == rc and gc != 0 and first_error(ge, root / "go") == first_error(re, root / "rust"):
                kind = "error-match"
            elif gc == rc == 0:
                kind = graph.diverges(source, layout, flags, rust, go) or "rendered-match"
            else:
                kind = "error-diff" if gc == rc else "exit-diff"
        small = None
        if minimize and (kind == "output-diff" or gc == 0 and rc != 0):
            small = graph.minimize(source, layout, flags, rust, go,
                                   "output-diff" if rc == 0 else f"rust-exit-{rc}")
        results.append({"case": index, "layout": layout, "kind": kind,
                        "go_exit": gc, "rust_exit": rc, "minimized": small})
    return index, source, results


def generate(rng, family):
    direction = rng.choice(graph.DIRECTIONS)
    lines = [f"direction: {direction}"]
    if family == "containers":
        for name in ("a", "b"):
            lines += [f"{name}: {{", f"  direction: {rng.choice(graph.DIRECTIONS)}",
                      f"  label.near: {rng.choice(graph.LABEL_POSITIONS)}"]
            for i in range(rng.randint(1, 3)):
                lines.append(f"  n{i}: {{shape: {rng.choice(graph.SHAPES)}}}")
            lines.append("}")
        endpoints = ["a", "a.n0", "b", "b.n0", "outside"]
    elif family == "grids":
        rows = rng.choice(("grid-rows", "grid-columns"))
        lines += [f"g: {{ {rows}: {rng.randint(1, 3)}",
                  f"  grid-gap: {rng.choice((0, 1, 15, 40))}",
                  f"  label.near: {rng.choice(graph.LABEL_POSITIONS)}"]
        for i in range(rng.randint(2, 5)):
            lines.append(f"  n{i}: {{width: {rng.choice((31, 80, 201))}; height: {rng.choice((20, 41, 100))}}}")
        lines += ["}", f"legend: {{near: {rng.choice(graph.NEAR)}}}", "external"]
        endpoints = ["g.n0", "g.n1", "external"]
    elif family == "sequences":
        lines += ["s: {", "  shape: sequence_diagram", "  a", "  b", "  c"]
        for _ in range(rng.randint(2, 7)):
            a, b = rng.choices(("a", "b", "c", "a.span"), k=2)
            lines.append(f"  {a} -> {b}: {rng.choice(('call', 'reply', 'long response label'))}")
        if rng.random() < 0.5:
            lines.append("  b.note: explanatory note")
        lines += ["}", "external", "other: {child}"]
        endpoints = ["s", "external", "other.child"]
    elif family == "ports":
        lines += ["db: {shape: sql_table; id: int {constraint: primary_key}; owner: int; name: text}",
                  'api: {shape: class; "+field": int; "-method()": bool}',
                  f"host: {{n: {{shape: {rng.choice(graph.SHAPES)}}}}}"]
        endpoints = ["db.id", "db.owner", "db.name", "api", "host.n", "host"]
    else:
        lines += ["container: {", f"  direction: {rng.choice(graph.DIRECTIONS)}"]
        for i in range(rng.randint(2, 5)):
            lines.append(f"  n{i}: {{shape: {rng.choice(graph.SHAPES)}; width: {rng.choice((21, 41, 83, 120))}; height: {rng.choice((19, 25, 31, 41, 100))}; label.near: {rng.choice(graph.LABEL_POSITIONS)}}}")
        lines.append("}")
        endpoints = ["container.n0", "container.n1", "container", "outside"]
    for _ in range(rng.randint(1, 6)):
        a, b = rng.choices(endpoints, k=2)
        label = rng.choice(("", ": x", ": a longer connection label"))
        lines.append(f"{a} {rng.choice(graph.ARROWS)} {b}{label}")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("rust")
    parser.add_argument("--go", default=str(graph.references.GO_ORACLE))
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--cases", type=int, default=1000, help="cases per family")
    parser.add_argument("--families", default=",".join(FAMILIES))
    parser.add_argument("--jobs", type=int, default=5)
    parser.add_argument("--no-minimize", action="store_true")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    rng = random.Random(args.seed)
    summary = []
    for family in args.families.split(","):
        if family not in FAMILIES:
            parser.error(f"unknown family: {family}")
        cases = [(i, generate(rng, family), ()) for i in range(args.cases)]
        counts = {}
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            def check(case):
                return compare(case, args.rust, args.go, not args.no_minimize)
            for index, source, findings in pool.map(check, cases):
                for finding in findings:
                    kind = finding["kind"]
                    counts[kind] = counts.get(kind, 0) + 1
                    if kind in ("rendered-match", "error-match"):
                        continue
                    stem = args.out / f"{family}-{index:05d}-{finding['layout']}"
                    stem.with_suffix(".d2").write_text(finding["minimized"] or source)
                    stem.with_suffix(".original.d2").write_text(source)
                    stem.with_suffix(".json").write_text(json.dumps(finding, indent=2) + "\n")
        summary.append({"family": family, "seed": args.seed, "jobs": 2 * len(cases), **counts})
        print(json.dumps(summary[-1]), flush=True)
        (args.out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    return int(any(set(row) - {"family", "seed", "jobs", "rendered-match", "error-match", graph.NONDETERMINISTIC}
                   for row in summary))


if __name__ == "__main__":
    raise SystemExit(main())
