"""Request generators for the d2svgimport capture hooks (tools/fixtures/mathtext-svg.toml).

    fixture-gen-svg.py geometry|path|use-graph

writes $D2RS_RECIPE_OUTPUT: the JSON request list read by
tools/fixtures/hooks/mathtext-svg/d2rs_<mode>_test.go. The hand-written
cases and sweeps reproduce the v0.9.0 capture's requests; its randomized
sections (100 random relative paths, 100 random <use> graphs) are drawn
from a fixed-seed PRNG of the same shape, so their concrete values differ.
"""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

LENGTH_VALUES = ["-0", "1mm", "-1", "auto", " auto ", " " * 17 + "auto", "1%", "1e999", "bad", ""]
POINT_DATA = ["", "0 0", "0 0 1 2", "0 0 1 2 3", "0,0,1,2", "0,,0", ",0 0", "0 0,", "0-1+2-3", "1e-2 3E+2 4 5", "1mm 2",
              "0\u00a00", "bad", "M0 0", "M0 0M1 1", "M0 0Z", "M0 0A1 1 0 0 1 0 0", "M0 0L1 1", "M,0 0"]
# (maximum path commands, already-used commands) charged before each geometry.
GEOMETRY_PROFILES = [(0, 0), (1, 0), (2, 0), (3, 1), (100, 0), (100, 100)]


def geometry() -> list[dict]:
    cases: list[tuple[str, dict[str, str]]] = []
    for name in ["svg", "g", "defs", "style", "use", "clipPath", "rect", "circle", "ellipse", "line", "path", "polyline", "polygon"]:
        cases.append((name, {}))
    for name, attrs in [("rect", ["x", "y", "width", "height", "rx", "ry"]), ("circle", ["cx", "cy", "r"]),
                        ("ellipse", ["cx", "cy", "rx", "ry"]), ("line", ["x1", "y1", "x2", "y2"])]:
        for attr in attrs:
            for value in LENGTH_VALUES:
                cases.append((name, {attr: value}))
    box = {"height": "20", "width": "10", "x": "1", "y": "2"}
    for radii in [{"rx": "20"}, {"ry": "30"}, {"rx": "auto", "ry": "30"}, {"rx": "4", "ry": "8"}, {"rx": "-1", "ry": "bad"}]:
        cases.append(("rect", dict(sorted({**box, **radii}.items()))))
    for data in POINT_DATA:
        cases += [("path", {"d": data}), ("polyline", {"points": data}), ("polygon", {"points": data})]
    rows = [{"cancel": "", "name": name, "attrs": attrs, "maximum": maximum, "used": used, "max_bytes": 10000}
            for name, attrs in cases for maximum, used in GEOMETRY_PROFILES]
    # Context checks precede (and survive) attribute and budget errors.
    cancelled = [("path", {}), ("path", {"d": " " * 9000}), ("path", {"d": "M0 0Z"}), ("polyline", {}), ("polyline", {"points": "bad"}),
                 ("polyline", {"points": "0 0"}), ("polygon", {"points": "0 0"}), ("rect", {"width": "bad"}), ("line", {}), ("line", {"x1": "bad"})]
    for name, attrs in cancelled:
        for mode in ("cancelled", "deadline"):
            rows.append({"cancel": mode, "name": name, "attrs": attrs, "maximum": 100, "used": 0, "max_bytes": 10000})
    return rows


PATH_SYNTAX = ["", " ", "M0 0", "m1 2 3 4 5 6", "M0 0Z", "M0 0Zz", "M0 0Z1 2", "Z", "L1 2", "C1 2 3 4 5 6", "M,0 0", "M0,,0", "M0 0,",
               "M0 0X1 2", "M0 0\u00a0L1 2", "M1e 0", "M. 0", "M1e999 0", "M1e-999 0", "M0 0L1e308 1e308l1e308 1e308",
               "M0 0Q1e308 1e308 0 0T1 1", "M0 0A-1 2 0 0 1 1 1", "M0 0A1 2 0 2 1 1 1", "M0 0A1 2 0 0 1 0 0", "M0 0A0 2 0 0 1 1 1",
               "M1 2C3 4 5 6 7 8S9 10 11 12Q13 14 15 16T17 18Zs1 2 3 4", "M0 0A1 2 0 011 2", "M0 0H2V3h-1v-2", "M0 0L.1.2-.3+.4",
               "M1e+2 3E-2 4 5", "M0 0\x0cL1 2"]
PATH_ARGS = {"M": "1 2", "L": "1 2", "H": "1", "V": "1", "C": "1 2 3 4 5 6", "S": "1 2 3 4", "Q": "1 2 3 4", "T": "1 2", "A": "1 2 30 0 1 3 4"}
RANDOM_SEGMENTS = {"l": 2, "h": 1, "v": 1, "c": 6, "s": 4, "q": 4, "t": 2}


def path() -> list[dict]:
    data = list(PATH_SYNTAX)
    for upper, args in PATH_ARGS.items():
        for command in (upper, upper.lower()):
            data += [f"M2 3{command}{args}", f"{command}{args}", f"M2 3{command}{args} {args}", f"M2 3{command}{args}Z{command}{args}"]
    for rotation in ["180", "540", "900", "-180", "-540", "-900", "1e308", "-1e308", "1e-308"]:
        data.append(f"M0 0A1 2 {rotation} 0 1 3 4")
    rng = random.Random(20261007)
    for _ in range(100):
        parts = ["M0 0"]
        for _ in range(rng.randint(1, 12)):
            command = rng.choice(sorted(RANDOM_SEGMENTS))
            values = " ".join(f"{rng.uniform(-10, 10):.1f}" for _ in range(RANDOM_SEGMENTS[command]))
            parts.append(command + values)
        data.append("".join(parts))
    rows = [{"data": d, "max_bytes": 10000, "max_commands": m} for d in data for m in (1, 2, 5, 1000)]
    # One byte under each syntax case's UTF-8 length, then both zero limits.
    rows += [{"data": d, "max_bytes": max(len(d.encode()) - 1, 1), "max_commands": 1000} for d in PATH_SYNTAX]
    rows += [{"data": "M0 0", "max_bytes": 0, "max_commands": 1}, {"data": "M0 0", "max_bytes": 1, "max_commands": 0}]
    return rows


def group(children_uses: list[int | None | str]) -> list[dict]:
    """Root <g> containing one element per entry: an int/None <use> target, or "g"."""
    out = [{"children": list(range(1, len(children_uses) + 1)), "is_use": False, "target": None}]
    for item in children_uses:
        if item == "g":
            out.append({"children": [], "is_use": False, "target": None})
        else:
            out.append({"children": [], "is_use": True, "target": item})
    return out


def use_graph() -> list[dict]:
    g = lambda children=(): {"children": list(children), "is_use": False, "target": None}  # noqa: E731
    u = lambda target: {"children": [], "is_use": True, "target": target}  # noqa: E731
    graphs = [[g()], [g([1]), u(None)], [g([1]), u(1)], [g([1, 2]), u(2), u(1)], [g([1, 2]), u(None), u(2)], [g([1, 2]), u(1), u(None)],
              [g([1, 2, 3]), g([2]), g(), u(1)], [g([1, 2, 3]), g([2]), g(), u(0)]]
    # Chains of n uses ending at a group, around the depth limits.
    for n in (1, 2, 3, 10, 100, 4096):
        graphs.append(group([k + 2 for k in range(n)] + ["g"]))
    rng = random.Random(20261007)
    for _ in range(100):
        size = rng.randint(2, 29)
        graphs.append(group([None if rng.random() < 0.05 else rng.randrange(size) for _ in range(size - 1)]))
    return [{"elements": elements, "maximum_depth": depth} for elements in graphs for depth in (0, 1, 2, 3, 10, 10000)]


def main() -> None:
    modes = {"geometry": geometry, "path": path, "use-graph": use_graph}
    if len(sys.argv) != 2 or sys.argv[1] not in modes:
        sys.exit(f"usage: {sys.argv[0]} {'|'.join(modes)}")
    rows = modes[sys.argv[1]]()
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(json.dumps(rows, separators=(",", ":"), ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
