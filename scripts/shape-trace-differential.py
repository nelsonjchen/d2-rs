#!/usr/bin/env python3
"""Seeded differential sweep of edge endpoints on non-rectangular shapes.

usage: shape-trace-differential.py <d2-rs> [--go GO] [--cases 400] [--seed 1]
       [--layouts dagre,elk] [--jobs 8] [--out DIR]

Dagre and ELK clip each edge to its endpoint shape's outline with Go's
lib/shape.TraceToShapeBorder. Its edge cases (tiny parallelograms and steps,
vertical rays into ellipses, rays grazing a vertex, cubic outlines) depend on
exact shape sizes and edge angles, which the upstream corpus barely varies.
Each case is a small graph of non-rectangular shapes with varied sizes,
directions, parallel and reversed edges and containers; both binaries render
it with every layout and complete output inventories are compared byte for
byte. Mismatching inputs are written under --out with a JSONL index; the exit
status is 1 when any case differs.
"""
import argparse
import concurrent.futures
import json
import random
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import references  # noqa: E402
from fuzz_common import run  # noqa: E402

# Every shape lib/shape traces (rectangular shapes return the border point).
SHAPES = ("circle", "oval", "cloud", "diamond", "hexagon", "parallelogram", "step", "callout",
          "cylinder", "queue", "person", "stored_data", "page", "document", "package", "c4-person")
# Sizes around the parallelogram (26 px) and step (35 px) wedge thresholds,
# plus very wide and very tall boxes that make near-vertical and
# near-horizontal rays.
SIZES = (None, None, None, 10, 20, 25, 26, 27, 34, 35, 36, 52, 70, 140, 300)
DIRECTIONS = (None, "right", "down", "left", "up")
ARROWS = ("->", "->", "<-", "<->", "--")


def generate(rng):
    """One small D2 graph that routes edges into non-rectangular shapes."""
    lines = []
    direction = rng.choice(DIRECTIONS)
    if direction:
        lines.append(f"direction: {direction}")
    count = rng.randint(2, 5)
    names = [f"n{i}" for i in range(count)]
    container = rng.random() < 0.25
    for name in names:
        attrs = [f"shape: {rng.choice(SHAPES)}"]
        width, height = rng.choice(SIZES), rng.choice(SIZES)
        if width:
            attrs.append(f"width: {width}")
        if height:
            attrs.append(f"height: {height}")
        if rng.random() < 0.3:
            attrs.append(f'label: "{"x" * rng.randint(1, 24)}"')
        lines.append(f"{name}: {{ {'; '.join(attrs)} }}")
    if container:
        inside = rng.sample(names, rng.randint(1, count - 1))
        lines.append("box: {")
        lines.extend(f"  {name}" for name in inside)
        lines.append("}")
        qualified = {name: f"box.{name}" if name in inside else name for name in names}
    else:
        qualified = {name: name for name in names}
    for _ in range(rng.randint(1, 2 * count)):
        source, target = rng.sample(names, 2)
        label = f": e{rng.randint(0, 99)}" if rng.random() < 0.3 else ""
        lines.append(f"{qualified[source]} {rng.choice(ARROWS)} {qualified[target]}{label}")
    return "\n".join(lines) + "\n"


def compare(case, rust, go, layouts):
    index, source = case
    mismatches = []
    with tempfile.TemporaryDirectory(prefix="shape-trace-") as tmp:
        root = Path(tmp)
        for layout in layouts:
            outcomes = {}
            for name, binary in (("go", go), ("rust", rust)):
                directory = root / f"{layout}-{name}"
                directory.mkdir()
                (directory / "input.d2").write_text(source)
                outcomes[name] = run(binary, directory, layout, "out")
            go_code, _, go_digest = outcomes["go"]
            rust_code, _, rust_digest = outcomes["rust"]
            if go_code != 0:
                continue  # no oracle result
            if rust_code != 0 or go_digest != rust_digest:
                mismatches.append({"case": index, "layout": layout, "go": go_code, "rust": rust_code})
    return index, source, mismatches


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("rust")
    parser.add_argument("--go", default=str(references.GO_ORACLE))
    parser.add_argument("--cases", type=int, default=400)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--layouts", default="dagre,elk")
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument("--out", default=str(references.REPORTS / "shape-trace"))
    args = parser.parse_args(argv)
    layouts = [layout for layout in args.layouts.split(",") if layout]
    rng = random.Random(args.seed)
    cases = [(i, generate(rng)) for i in range(args.cases)]
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    findings = []
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        for index, source, mismatches in pool.map(lambda c: compare(c, args.rust, args.go, layouts), cases):
            for mismatch in mismatches:
                path = out / f"case-{index:05d}-{mismatch['layout']}.d2"
                path.write_text(source)
                findings.append({**mismatch, "input": str(path)})
    with open(out / "findings.jsonl", "w") as index_file:
        for finding in sorted(findings, key=lambda f: (f["case"], f["layout"])):
            index_file.write(json.dumps(finding) + "\n")
    for layout in layouts:
        bad = sum(1 for f in findings if f["layout"] == layout)
        print(f"{layout}: {args.cases - bad}/{args.cases} cases match Go")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
