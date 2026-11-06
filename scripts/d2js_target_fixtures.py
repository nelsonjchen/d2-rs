#!/usr/bin/env python3
"""Raw d2target render fixtures from the pinned d2js (recipe `d2js-target`).

Runs ``tools/d2js-capture/target.mjs`` against the `d2js-node-package` build,
then shapes its raw observations (SVG text or error per case) into the eight
fixtures read by d2-rs-render, d2-rs-export and d2-rs-wasm. The derived
fields (hash class, viewBox, <text> contents/origins, legend texts, animation
frames, root/inner svg attributes) are computed from the Go SVG exactly as the
v0.9.0-era captures recorded them; nothing is normalized.

    uv run --no-project python scripts/regen-fixtures.py --only d2js-target
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import xml.etree.ElementTree as ET

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
NS = "{http://www.w3.org/2000/svg}"
LEGEND = ["edge-empty-default", "edge-hidden-default", "edge-shape-default", "edge-connection-default",
          "edge-mixed-default", "edge-lower-default", "edge-upper-default", "edge-structured-default",
          "edge-effects-default", "edge-threeDee-default", "edge-blend-default", "edge-image-default",
          "edge-arrows-default", "edge-discarded-default", "container-mixed-sketch", "class-structured-default",
          "boards-mixed-terminal"] + [f"{host}-{kind}-no-id{suffix}" for host in ["edge", "container", "class", "boards"]
                                      for kind in ["shape", "connection"] for suffix in ["", "-classes"]]
# (group, source name, fixture name) selected for raw-target-render-v090.json.
BOUNDARY = ([("mono", "rich-default-HandDrawn-SourceSansPro-true"), ("mono", "rich-mono-SourceSansPro-HandDrawn-false"),
             ("paint", "class-default"), ("paint", "class-terminal")]
            + [("target", "edge-" + m) for m in ["clone", "translate", "paint", "label"]]
            + [("boundary", "options-edge-" + t) for t in ["default", "size", "salt", "appendix", "ignored-compile-fields"]]
            + [("boundary", "options-selected-board")]
            + [("boundary", "error-" + t) for t in ["missing", "invalid-diagram", "pad-type", "pad-fraction", "scale-type",
                                                     "sketch-type", "target-missing", "target-all", "target-parse"]]
            + [("boundary", n) for n in ["terminal-appendix-300", "terminal-appendix-301", "appendix-without-rows"]]
            + [("legend", n) for n in LEGEND]
            + [("ascii", n) for n in ["empty-extended", "shape-rectangle-unicode-extended", "shape-document-plain-standard",
                                      "shape-class-outside-extended", "edge-clone-standard-2", "edge-clone-STANDARD-0.5",
                                      "diagonal-false-diamond-triangle-c3a9c2b2f09f9982-extended",
                                      "class-140-3-BORDER_TOP_CENTER-standard"]])
OUT = {
    "fonts": "crates/d2-rs-render/tests/fixtures/public-target-fonts-v090.json",
    "mono": "crates/d2-rs-render/tests/fixtures/public-target-mono-fonts-v090.json",
    "paint": "crates/d2-rs-render/tests/fixtures/public-target-paint-v090.json",
    "legend": "crates/d2-rs-render/tests/fixtures/target-legend-source-v090.json",
    "ascii": "crates/d2-rs-export/tests/fixtures/target-ascii-source-v090.json",
    "boundary": "crates/d2-rs-wasm/tests/fixtures/raw-target-render-v090.json",
    "sdk": "crates/d2-rs-wasm/tests/fixtures/target-sdk-source-v090.json",
    "shape_arrays": "crates/d2-rs-wasm/tests/fixtures/target-shape-arrays-source-v090.json",
}


def sha(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


def main_hash(svg: str) -> str:
    return re.search(r'class="(d2-\d+)', svg).group(1)


def texts(root):
    return [e for e in root.iter(NS + "text")]


def content(node) -> str:
    return "".join(node.itertext())


def font_case(row, structured_prefixes=("class-", "table-")):
    svg = row["svg"]
    root = ET.fromstring(svg)
    nodes = texts(root)
    structured = row["name"].startswith(structured_prefixes)
    return {"name": row["name"], "diagram": row["diagram"], "sketch": row["options"]["sketch"],
            "expected_hash": main_hash(svg), "expected_view_box": root.get("viewBox"),
            "expected_texts": [content(e) for e in nodes],
            "structured_xy": [[e.get("x"), e.get("y")] for e in nodes] if structured else None,
            "source_svg_sha256": sha(svg)}


def paint_case(row):
    svg = row["svg"]
    root = ET.fromstring(svg)
    classes = [g.get("class") for g in root.iter(NS + "g") if g.get("class") and " custom-" in g.get("class")]
    return {"name": row["name"], "diagram": row["diagram"], "options": row["options"],
            "expected_hash": main_hash(svg), "expected_view_box": root.get("viewBox"),
            "expected_texts": [content(e) for e in texts(root)], "expected_classes": classes}


def legend_case(row):
    svg = row["svg"]
    root = ET.fromstring(svg)
    # d2svg.RenderLegend writes `<text class=... x=... y=...>`; diagram labels lead with x.
    legend = [{"text": content(e), "x": e.get("x"), "y": e.get("y"), "cls": e.get("class")}
              for e in texts(root) if next(iter(e.attrib), None) == "class"]
    return {"name": row["name"], "diagram": row["diagram"], "options": row["options"], "source_svg_sha256": sha(svg),
            "main_hash": main_hash(svg), "viewbox": root.get("viewBox"), "legend_text": legend}


def ascii_case(row):
    out = {"name": row["name"], "diagram": row["diagram"], "options": row["options"]}
    if "error" in row:
        out["error"] = row["error"]
    else:
        out["output"] = row["svg"]
        out["sha256"] = sha(row["svg"])
    return out


def boundary_case(group, row):
    prefix = {"mono": "mono-", "paint": "paint-", "target": "target-", "legend": "legend-", "ascii": "ascii-"}.get(group, "")
    out = {"name": prefix + row["name"], "diagram": row["diagram"], "options": row["options"]}
    if "error" in row:
        out["error"] = row["error"]
    elif row["options"].get("ascii"):
        out["output"] = row["svg"]
        out["source_sha256"] = sha(row["svg"])
    else:
        svg = row["svg"]
        out.update(hash=main_hash(svg), viewBox=ET.fromstring(svg).get("viewBox"), xml=svg.startswith("<?xml"),
                   source_svg_sha256=sha(svg))
    return out


def svg_attributes(node):
    return None if node is None else {k: node.get(k) for k in ["width", "height", "viewBox", "preserveAspectRatio"]}


def sdk_observation(svg: str, options) -> dict:
    root = ET.fromstring(svg)
    inner = next((c for c in root if c.tag == NS + "svg"), None)
    frames = [{"hash": h, "index": int(i), "total_ms": int(ms)}
              for h, i, ms in re.findall(r"animation: d2Transition-(d2-\d+)-(\d+) (\d+)ms infinite", svg)]
    out = {"attributes": svg_attributes(root), "inner_attributes": svg_attributes(inner),
           "xml": svg.startswith("<?xml"), "frames": frames}
    if not options.get("animateInterval"):
        out["hash"] = main_hash(svg)
    return out


def sdk_case(row):
    out = {"name": row["name"], "diagram": row["diagram"], "options": row["options"]}
    if "error" in row:
        out["error"] = row["error"]
        out["source_svg_sha256"] = None
        return out
    svg = row["svg"]
    if row["raw_data_null"]:
        # The worker decodes atob(null); the raw WASM response carries no SVG.
        out.update(sdk_output=svg, raw_output="", source_svg_sha256=sha(svg))
        return out
    out.update(sdk_observation(svg, row["options"]))
    out["source_svg_sha256"] = sha(svg)
    if row["diagram"] is not None and not row["diagram"].get("shapes") and not row["diagram"].get("layers"):
        root = ET.fromstring(svg)
        background = next(root.iter(NS + "rect"), None)
        if background is not None and background.get("fill") is None:
            out["root_omits_fill"] = True
    return out


def dump(logical: str, value, ascii_only: bool = True) -> None:
    # The historical files differ only in whether non-ASCII is escaped.
    fixture_recipe.output(logical).write_text(json.dumps(value, indent=2, ensure_ascii=ascii_only) + "\n")


def main() -> None:
    commit = os.environ.get("D2RS_CAPTURE_COMMIT") or json.loads((ROOT / "reference-manifest.json").read_text())["d2"]["commit"]
    raw_path = fixture_recipe.workdir() / "target-observations.json"
    subprocess.run(["node", str(ROOT / "tools/d2js-capture/target.mjs"), str(raw_path), str(ROOT / "tools/d2js-capture/inputs")],
                   check=True)
    raw = json.loads(raw_path.read_text())
    groups: dict[str, list] = {}
    for row in raw["cases"]:
        groups.setdefault(row["group"], []).append(row)
    index = {(g, r["name"]): r for g, rows in groups.items() for r in rows}

    dump(OUT["fonts"], {"source_commit": commit, "source": "d2svg.Render / d2fonts embedding and d2sketch.Table/Class",
                        "cases": [font_case(r) for r in groups["fonts"]]})
    dump(OUT["mono"], {"source_commit": commit, "cases": [font_case(r) for r in groups["mono"]]})
    dump(OUT["paint"], {"provenance": f"Pinned Go D2 {commit}. Render-only options applied to serialized default-theme "
                                      "targets; source capture script tools/d2js-capture/target.mjs (paint).",
                        "cases": [paint_case(r) for r in groups["paint"]]})
    dump(OUT["legend"], {"source_commit": commit, "source": "d2svg.RenderLegend / renderLegendShapeIcon / renderLegendConnectionIcon",
                         "cases": [legend_case(index[("legend", n)]) for n in LEGEND]})
    dump(OUT["ascii"], [ascii_case(r) for r in groups["ascii"]])
    dump(OUT["boundary"], {"source_commit": commit, "cases": [boundary_case(g, index[(g, n)]) for g, n in BOUNDARY]})
    inheritance = raw["inheritance"]
    inherit_cases = []
    for i, row in enumerate(inheritance["rows"]):
        case = {"name": f"compile-target-render-{i}", "options": row["options"]}
        if "error" in row:
            case["error"] = row["error"]
        else:
            case.update(sdk_observation(row["svg"], row["options"]))
        inherit_cases.append(case)
    dump(OUT["sdk"], {"source_commit": commit, "source_capture": "tools/d2js-capture/target.mjs (sdk)",
                      "cases": [sdk_case(r) for r in groups["sdk"]],
                      "inheritance": {"input": inheritance["input"], "compile_options": inheritance["compile_options"],
                                      "cases": inherit_cases}})
    dump(OUT["shape_arrays"], {"source_commit": commit, "scope": "Complete shape arrays/all fields/order across source "
                               "Dagre,TALA,ELK;two exact source errors retained;not whole diagrams/root/connections/API parity.",
                               "cases": raw["shape_arrays"]}, ascii_only=False)


if __name__ == "__main__":
    main()
