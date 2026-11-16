#!/usr/bin/env python3
"""Seeded Dagre/ELK feature-combination hunt, with complete output inventories.

Reuses graph-differential's capture, map-seed resampling and minimization.
Generated inputs and oracle outputs are temporary; only findings are retained.
A case whose Go and d2-rs outcomes (inventory digest, or exit code and first
error line) differ is resampled with the seedable-map oracle and classified by
scripts/mapseed.py: mapseed-match and known-mapseed pass, as do
rendered-match and error-match; go-nondeterministic-no-rust-match,
mapseed-oracle-missing and every other kind fail. Each finding's JSON carries
the classification, the matching map seeds and the Go variant count.
"""
import argparse
import concurrent.futures
import importlib.util
import json
from pathlib import Path
import random
import sys
import tempfile

from fuzz_common import first_error

_spec = importlib.util.spec_from_file_location(
    "graph_differential", Path(__file__).with_name("graph-differential.py"))
graph = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(graph)

FAMILIES = ("containers", "grids", "sequences", "ports", "short-shapes")


PASSING = {"rendered-match", "error-match", graph.mapseed.MAPSEED_MATCH, graph.mapseed.KNOWN_MAPSEED}


def compare(case, rust, go, resampler, minimize=True):
    index, source, flags = case
    results = []
    for layout in ("dagre", "elk"):
        record = {}
        with tempfile.TemporaryDirectory(prefix="layout-hunt-") as tmp:
            root = Path(tmp)
            go_result = graph.outcome(go, source, layout, flags, root, "go")
            rust_result = graph.outcome(rust, source, layout, flags, root, "rust")
            (gc, ge, gh), (rc, re, rh) = go_result, rust_result
            go_key = graph.outcome_key(go_result, root / "go")
            rust_key = graph.outcome_key(rust_result, root / "rust")
            if "panic:" in ge or isinstance(gc, int) and gc < 0:
                kind = "go-crash"
            elif gc == "timeout" or rc == "timeout":
                kind = "timeout"
            elif gc == "invalid-output" or rc == "invalid-output":
                kind = "invalid-output"
            else:
                if gc == rc == 0:
                    kind = "rendered-match" if gh == rh else "output-diff"
                elif gc == rc and first_error(ge, root / "go") == first_error(re, root / "rust"):
                    kind = "error-match"
                else:
                    kind = "error-diff" if gc == rc else "exit-diff"
                if kind not in ("rendered-match", "error-match") or resampler.known(source, layout, flags):
                    resolution = resampler.resolve(source, layout, flags, root, go_key, rust_key)
                    record = resolution.record()
                    if resolution.classification == graph.mapseed.MATCH:
                        pass  # A known input that stock Go matched.
                    elif resolution.classification != graph.mapseed.MISMATCH:
                        kind = resolution.classification
        small = None
        if minimize and (kind in ("output-diff", graph.mapseed.NO_RUST_MATCH) and rc == 0 or gc == 0 and rc != 0):
            small = graph.minimize(source, layout, flags, rust, go, kind if rc == 0 else f"rust-exit-{rc}", resampler)
        results.append({"case": index, "layout": layout, "kind": kind,
                        "go_exit": gc, "rust_exit": rc, "minimized": small, **record})
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
    graph.mapseed.add_arguments(parser)
    args = parser.parse_args()
    registry = graph.mapseed.Registry(args.mapseed_registry)
    resampler = graph.Resampler(args.go, args.go_mapseed, args.mapseed_seeds, registry)
    recorded = []
    args.out.mkdir(parents=True, exist_ok=True)
    rng = random.Random(args.seed)
    summary = []
    for family in args.families.split(","):
        if family not in FAMILIES:
            parser.error(f"unknown family: {family}")
        cases = [(i, generate(rng, family), ()) for i in range(args.cases)]
        counts, family_records = {}, []
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            def check(case):
                return compare(case, args.rust, args.go, resampler, not args.no_minimize)
            for index, source, findings in pool.map(check, cases):
                for finding in findings:
                    kind = finding["kind"]
                    counts[kind] = counts.get(kind, 0) + 1
                    family_records.append({"layout": finding["layout"], "mapseed_seeds": finding.get("mapseed_seeds", []),
                                           "classification": "match" if kind in ("rendered-match", "error-match") else kind})
                    if finding.get("go_variants", 1) > 1:
                        recorded.append((resampler.identity(source, finding["layout"], ()), source,
                                         graph.Resampler.args(finding["layout"], ()),
                                         f"layout-hunt.py --seed {args.seed} --families {family} case {index}"
                                         f" layout {finding['layout']}",
                                         graph.mapseed.Resolution(kind, finding.get("mapseed_seeds", []),
                                                                  finding["go_variants"])))
                    if kind in ("rendered-match", "error-match"):
                        continue
                    stem = args.out / f"{family}-{index:05d}-{finding['layout']}"
                    stem.with_suffix(".d2").write_text(finding["minimized"] or source)
                    stem.with_suffix(".original.d2").write_text(source)
                    stem.with_suffix(".json").write_text(json.dumps(finding, indent=2) + "\n")
                    if kind not in PASSING:
                        print(f"FAIL {kind}: {stem.with_suffix('.d2')}", flush=True)
        summary.append({"family": family, "seed": args.seed, "jobs": 2 * len(cases), **counts})
        print(json.dumps(summary[-1]), flush=True)
        for layout in ("dagre", "elk"):
            print(graph.mapseed.summary(f"{family} {layout}", len(cases),
                                        [r for r in family_records if r["layout"] == layout]), flush=True)
        (args.out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    if any(graph.mapseed.ORACLE_MISSING in row for row in summary):
        print(f"error: {resampler.oracle.message()}", file=sys.stderr)
    if args.record_mapseed:
        added = graph.mapseed.record_cases(registry, recorded, "layout-hunt")
        print(f"recorded {added} new map-order-dependent inputs in {registry.path}")
    return int(any(set(row) - {"family", "seed", "jobs", *PASSING} for row in summary))


if __name__ == "__main__":
    raise SystemExit(main())
