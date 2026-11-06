#!/usr/bin/env python3
"""Generators for d2lib-compiled crates/d2-rs-render probe fixtures
(tools/fixtures/renderexport-render-probes.toml).

    render_probe_fixtures.py <family>

Each family builds its case list (hand-written D2 sources from
tools/fixtures/seeds/render-probes.json, small grids expanded here), runs the
tools/fixtures/hooks/render-compile-probe hook (d2lib.Compile with TALA seed 1
plus the requested SVG/scene/predicate extras) against $D2RS_UPSTREAM, and
writes the fixture layout its Rust reader expects. Run through
`uv run --no-project python scripts/regen-fixtures.py --only 'render-probe-*'`
(docs/FIXTURES.md).
"""
from __future__ import annotations

import itertools
import json
import os
import re
from pathlib import Path
import sys
import xml.etree.ElementTree as ET

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402
from render_oracle_fixtures import go_test_hook  # noqa: E402

REPO = Path(__file__).resolve().parents[1]
FIXTURES = "crates/d2-rs-render/tests/fixtures/"
HOOK = "tools/fixtures/hooks/render-compile-probe/d2rs_compile_probe_test.go"
SEEDS = json.loads((REPO / "tools/fixtures/seeds/render-probes.json").read_text())


def run(cases: list[dict], test: str = "TestD2RSCompileProbe") -> list[dict]:
    work = fixture_recipe.workdir()
    request, raw = work / f"{test}-cases.json", work / f"{test}-rows.json"
    request.write_text(json.dumps(cases, ensure_ascii=False))
    go_test_hook("d2lib", HOOK, test, {"D2RS_RECIPE_INPUT": str(request), "D2RS_RECIPE_OUTPUT": str(raw)})
    rows = json.loads(raw.read_text())
    assert len(rows) == len(cases), (len(rows), len(cases))
    return rows


def write(name: str, value, indent=None) -> None:
    kwargs = dict(indent=indent) if indent else dict(separators=(",", ":"))
    fixture_recipe.output(FIXTURES + name).write_text(json.dumps(value, ensure_ascii=False, **kwargs) + "\n")


def tag(el) -> str:
    return el.tag.rsplit("}", 1)[-1]


def svg_root(row: dict):
    assert "svg" in row, (row["name"], row.get("svg_error"))
    return ET.fromstring(row["svg"].encode())


def font_size(el):
    """Text size as the native renderer's font-size attribute (Go writes it in style)."""
    if el.get("font-size") is not None:
        return el.get("font-size")
    for decl in (el.get("style") or "").split(";"):
        key, _, value = decl.partition(":")
        if key.strip() == "font-size":
            return value.strip().removesuffix("px")
    return None


def upstream_text(path: str) -> str:
    return (Path(os.environ["D2RS_UPSTREAM"]) / path).read_text()


def seed_cases(family: str) -> list[dict]:
    out = []
    for case in SEEDS[family]:
        case = dict(case)
        if "upstream_source" in case:
            case["source"] = upstream_text(case.pop("upstream_source"))
        out.append(case)
    return out


# --- families ---------------------------------------------------------------

def appendix_bounds():
    """Raw two-shape targets with a connection detouring past one side."""
    rows = []
    sides = {"left": (-90, 60), "top": (60, -90), "right": (330, 60), "bottom": (60, 330)}
    for side, (mx, my) in sides.items():
        for stroke in (0, 1, 5):
            for offset in (0, -200):
                for profile in ("plain", "shadow", "multiple", "3d"):
                    p = lambda x, y: {"x": x + offset, "y": y + offset}
                    route = [p(60, 100), p(mx, my), p(190, 150)]
                    a = {"id": "a", "type": "rectangle", "pos": p(10, 20), "width": 100, "height": 80, "strokeWidth": stroke}
                    if profile != "plain":
                        a[profile] = True
                    b = {"id": "b", "type": "rectangle", "pos": p(150, 150), "width": 80, "height": 80, "strokeWidth": stroke, "tooltip": "details"}
                    rows.append({"name": f"{side}-{stroke}-{offset}-{profile}", "offset": offset, "profile": profile,
                                 "route": route, "stroke": stroke,
                                 "target": {"shapes": [a, b], "connections": [
                                     {"id": "edge", "src": "a", "dst": "b", "strokeWidth": stroke, "route": route}]}})
    write("appendix-bounds-source-v090.json", run(rows, "TestD2RSAppendixBounds"))


def markdown_background():
    cases = []
    md = "a: |md\n  #### Managers\n\n  - Clock\n  - Reset\n  - Power\n|\n"
    for prefix, shape in (("", "a.shape: rectangle\n"), ("text-", "")):
        for theme in (0, 7):
            for fill in ("default", "lightpink", "none", "transparent"):
                for opacity in ("1", "0.37"):
                    for size in (40, 50):
                        style = ("" if fill == "default" else f"fill: {fill}; ") + f"opacity: {opacity}; font-size: {size}"
                        cases.append({"name": f"{prefix}t{theme}-{fill}-o{opacity}-s{size}", "theme": theme,
                                      "source": md + "a.style: {" + style + "}\n" + shape, "extras": ["compile", "markdown"]})
    out = []
    for c, r in zip(cases, run(cases)):
        row = {"name": c["name"], "source": c["source"], "theme": c["theme"]}
        if "compile_error" in r:
            row["error"] = r["compile_error"]
        else:
            row.update({k: r[k] for k in ("shape", "markdown_transform", "clip", "background")})
        out.append(row)
    write("markdown-background-source-v090.json", out)


def root_canvas_style():
    shape = 'a: "" {width: 120; height: 80}\n'
    cases = [{"name": "default", "source": "\n" + shape}]
    i = 0
    for fill in ("black", "transparent", "beige", "linear-gradient(white, black)", "radial-gradient(white, black)"):
        for width in (0, 1, 6):
            for double in ("false", "true"):
                for radius in (0, 12, 400):
                    i += 1
                    cases.append({"name": f"matrix-{i}", "source": f"style: {{fill: {fill}; stroke: red; stroke-width: {width}; "
                                  f"double-border: {double}; border-radius: {radius}}}\n" + shape})
    for c in SEEDS["root-canvas-style"]:
        cases.append({"name": c["name"], "source": c["source"] + shape, "target": c.get("target", "")})
    probe = [{"name": c["name"], "source": c["source"], "inputPath": "root-style.d2", "layer": bool(c.get("target")),
              "extras": ["compile", "root"]} for c in cases]
    out = []
    for c, r in zip(cases, run(probe)):
        if "compile_error" in r:
            out.append({"name": c["name"], "source": c["source"], "compile_error": r["compile_error"]})
        elif "scene_error" in r:
            out.append({"name": c["name"], "source": c["source"], "target": c.get("target", ""), "root": r["root"], "error": r["scene_error"]})
        else:
            out.append({"name": c["name"], "source": c["source"], "target": c.get("target", ""), "root": r["root"],
                        "viewbox": r["viewbox"], "backgrounds": r["backgrounds"]})
    write("root-canvas-style-source-v090.json", out)


def connection_label_fill():
    cases = seed_cases("connection-label-fill")
    out = []
    for c, r in zip(cases, run([dict(c, extras=["compile", "svg"]) for c in cases])):
        if "compile_error" in r:
            out.append({"name": c["name"], "source": c["source"], "expected_error": True, "probe_stderr": r["compile_error"]})
            continue
        rects = [{k: el.get(k) for k in ("x", "y", "width", "height", "rx", "fill")}
                 for el in svg_root(r).iter() if tag(el) == "rect" and el.get("rx") == "10"]
        out.append({"name": c["name"], "source": c["source"], "target": r["diagram"], "backgrounds": rects})
    write("connection-label-fill-source-v090.json", out, indent=2)


def sequence_note_references():
    cases = seed_cases("sequence-note-references")
    out = []
    for c, r in zip(cases, run([dict(c, extras=["references"]) for c in cases])):
        refs = r["references"]
        row = {"name": c["name"], "source": c["source"]}
        if "error" in refs:
            row["error"] = refs["error"]
        else:
            row["objects"] = refs["objects"]
        out.append(row)
    write("sequence-note-references-source-v090.json", out, indent=2)


def sequence_groups():
    """sequence-group-reference, sequence-render-roles and sequence-group-label-fill."""
    cases = seed_cases("sequence-groups")
    rows = {c["name"]: r for c, r in zip(cases, run([dict(c, extras=["compile", "predicates", "references", "svg"]) for c in cases]))}
    by_name = {c["name"]: c for c in cases}

    def groups(r):
        return [p["id"] for p in r["predicates"] if p["group"]]

    reference = []
    for name in SEEDS["sequence-group-reference"]:
        r = rows[name]
        reference.append({"name": name, "source": by_name[name]["source"], "groups": groups(r),
                          "group_fills": {p["id"]: p["fill"] for p in r["predicates"] if p["group"]}})
    write("sequence-group-reference-source-v090.json", reference, indent=2)

    roles = []
    for name in SEEDS["sequence-render-roles"]:
        r = rows[name]
        roles.append({"name": name, "source": by_name[name]["source"], "shapes": r["diagram"]["shapes"],
                      "connections": r["diagram"]["connections"], "groups": groups(r),
                      "reference_edges": r["references"]["edges"]})
    write("sequence-render-roles-source-v090.json", roles, indent=2)

    name = SEEDS["sequence-group-label-fill"]
    r = rows[name]
    root = svg_root(r)
    parents = {child: parent for parent in root.iter() for child in parent}
    blend = next(el for el in root.iter() if el.get("class") == "shape blend")
    owner = parents[blend]
    rects = [{k: el.get(k) for k in ("x", "y", "width", "height", "fill")} for el in owner.iter() if tag(el) == "rect"]
    write("sequence-group-label-fill-source-v090.json",
          {"source": by_name[name]["source"], "shapes": r["diagram"]["shapes"], "group_rectangles": rects}, indent=2)


def sequence_extracted_labels():
    cases = seed_cases("sequence-extracted-labels")
    out = []
    for c, r in zip(cases, run([dict(c, theme=0, extras=["compile", "svg"]) for c in cases])):
        texts = [{"text": el.text or "", "x": el.get("x"), "y": el.get("y"), "font": font_size(el)}
                 for el in svg_root(r).iter() if tag(el) == "text"]
        out.append({"name": c["name"] + "-theme0", "source": c["source"], "target": r["diagram"], "texts": texts})
    write("sequence-extracted-labels-source-v090.json", out, indent=2)


def target_empty_config_bytes():
    """Hash-input bytes (d2target.Diagram.Bytes) of non-renderable empty targets."""
    cases = seed_cases("target-empty-config-bytes")
    out = []
    for c, r in zip(cases, run([dict(c, layout="dagre", extras=["compile", "bytes"]) for c in cases])):
        assert "bytes" in r, (c["name"], r.get("compile_error"))
        out.append({"name": c["name"], "source": c["source"], "bytes": r["bytes"]})
    write("target-empty-config-bytes-source-v090.json", out, indent=2)


def spore_sequence_lifeline_export():
    """Raw synthetic lifeline routes versus their exported target routes (ELK)."""
    case = dict(SEEDS["spore-sequence-lifeline-export"], extras=["compile", "raw_routes"])
    [r] = run([case])
    assert "diagram" in r, r.get("compile_error")
    raw = {e["id"]: e["route"] for e in r["raw_routes"]}
    out = [{"id": c["src"], "raw": raw[c["id"]], "target": c["route"]}
           for c in r["diagram"]["connections"] if "-lifeline-end-" in c["dst"]]
    assert out, "no lifelines exported"
    write("spore-sequence-lifeline-export-v090.json", out, indent=2)


def edge_icon_positions():
    """Connection.GetIconPosition per seed route x connection variant."""
    seed = json.loads((REPO / "tools/fixtures/seeds/render-edge-icon-positions.json").read_text())
    cases = []
    for i, route in enumerate(seed["routes"]):
        for v in seed["variants"]:
            connection = {k: val for k, val in route["connection"].items() if k not in v["drop"]}
            connection = {"route": connection.pop("route"), "iconPosition": v["set"].get("iconPosition"), **connection}
            connection.update(v["set"])
            cases.append({"name": f"route-{i}-{v['suffix']}", "raw_route": route["raw_route"], "has_icon": v["has_icon"],
                          "connection": connection})
    work = fixture_recipe.workdir()
    request, raw = work / "edge-icon-requests.json", work / "edge-icon-points.json"
    request.write_text(json.dumps([{"connection": c["connection"], "has_icon": c["has_icon"]} for c in cases]))
    go_test_hook("d2target", "tools/fixtures/hooks/render-edge-icon-positions/d2rs_edge_icon_positions_test.go",
                 "TestD2RSEdgeIconPositions", {"D2RS_RECIPE_INPUT": str(request), "D2RS_RECIPE_OUTPUT": str(raw)})
    points = json.loads(raw.read_text())
    assert len(points) == len(cases)
    write("edge-icon-positions-source-v090.json", {
        "source": "Pinned Go D2 Connection.GetIconPosition (tools/fixtures/hooks/render-edge-icon-positions). "
                  "Input routes are original source target float32/truncated3 values.",
        "cases": [dict(c, point=p) for c, p in zip(cases, points)]}, indent=2)


def class_header_underline():
    """d2lib (dagre) targets of class sources: header/row underline roles."""
    sources = SEEDS["class-header-underline"]
    rows = run([{"name": str(i), "source": s, "layout": "dagre", "extras": ["compile"]} for i, s in enumerate(sources)])
    cases = []
    for s, r in zip(sources, rows):
        assert "diagram" in r, (s, r.get("compile_error"))
        cases.append({"source": s, "diagram": r["diagram"]})
    commit = json.loads((REPO / "reference-manifest.json").read_text())["d2"]["commit"]
    write("class-header-underline-source-v090.json", {
        "source_commit": commit,
        "scope": "Original d2lib target metadata for28 class headers/row underline roles;full target or graph parity not asserted.",
        "cases": cases}, indent=2)


def cli_runs(directory: str, cases: list[dict], flags: list[str]) -> list[dict]:
    """Build the pinned upstream d2 CLI and export each case to PNG (scene path)."""
    import hashlib
    import subprocess
    work = fixture_recipe.workdir()
    binary = work / "d2"
    if not binary.exists():
        subprocess.run(["go", "build", "-mod=readonly", "-o", str(binary), "."], cwd=os.environ["D2RS_UPSTREAM"],
                       env=fixture_recipe.go_env(), check=True)
    rows = []
    for i, c in enumerate(cases):
        rel = Path(directory) / f"{i:03d}.d2"
        (work / rel).parent.mkdir(parents=True, exist_ok=True)
        (work / rel).write_text(c["source"])
        command = ["d2", *flags, str(rel), str(rel.with_suffix(".png"))]
        result = subprocess.run([str(binary), *command[1:]], cwd=work, capture_output=True, timeout=120,
                                env={**os.environ, "NO_COLOR": "1"})
        rows.append({"name": c["name"], "input": str(rel), "input_sha256": hashlib.sha256(c["source"].encode()).hexdigest(),
                     "command": command, "exit": result.returncode, "stdout": result.stdout.decode(),
                     # Success lines end with a wall-clock duration; keep the
                     # recipe deterministic (readers only compare failures).
                     "stderr": re.sub(r" in [0-9.]+(ns|µs|ms|s|m[0-9.]+s)\n", " in ELAPSED\n", result.stderr.decode())})
    return rows


def legend_connection_errors():
    cases = SEEDS["legend-connection-errors"]
    rows = cli_runs("legend", cases, ["--layout=dagre", "--timeout=30"])
    out = [{"name": r["name"], "input": r["input"], "input_sha256": r["input_sha256"], "command": r["command"],
            "exit": r["exit"], "timeout": False, "stdout": r["stdout"], "stderr": r["stderr"], "source": c["source"]}
           for c, r in zip(cases, rows)]
    write("legend-connection-errors-source-v090.json", out, indent=2)


def scene_label_admission():
    cases = SEEDS["scene-label-admission"]
    rows = cli_runs("admission", cases, ["--layout=dagre", "--timeout=30"])
    out = [{"name": r["name"], "input": c["source"], "source_exit": r["exit"], "source_error": r["stderr"] if r["exit"] else "",
            "source_file": r["input"]} for c, r in zip(cases, rows)]
    write("scene-label-admission-v090.json", out, indent=2)


FAMILIES = {
    "legend-connection-errors": legend_connection_errors,
    "scene-label-admission": scene_label_admission,
    "class-header-underline": class_header_underline,
    "edge-icon-positions": edge_icon_positions,
    "target-empty-config-bytes": target_empty_config_bytes,
    "spore-sequence-lifeline-export": spore_sequence_lifeline_export,
    "appendix-bounds": appendix_bounds,
    "markdown-background": markdown_background,
    "root-canvas-style": root_canvas_style,
    "connection-label-fill": connection_label_fill,
    "sequence-note-references": sequence_note_references,
    "sequence-groups": sequence_groups,
    "sequence-extracted-labels": sequence_extracted_labels,
}

if __name__ == "__main__":
    FAMILIES[sys.argv[1]]()
