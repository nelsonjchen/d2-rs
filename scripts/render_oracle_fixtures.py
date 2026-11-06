#!/usr/bin/env python3
"""Generators for the crates/d2-rs-render oracle fixtures (tools/fixtures/renderexport-render.toml).

Run through `uv run --no-project python scripts/regen-fixtures.py --only 'render-*'`
(docs/FIXTURES.md); every subcommand reads the D2RS_RECIPE_* environment.

  requests  input generator: expands tools/fixtures/seeds/render-markdown/*.json
            into the job list read by the lib/textmeasure hook
  scene     runs the d2scenebuild Markdown scene hook and writes
            markdown-scene-metadata and markdown-layout-viewport
  tooltip   runs the d2svg positioned-tooltip hook and adds the callout
            boxes, tails, texts and viewBoxes parsed from its SVGs

Seed files hold the hand-written Markdown snippets plus the request grid
(axes expanded outer-first; each request is named
`<prefix>-<snippet index>-<axis labels joined by '->`).
"""
from __future__ import annotations

import itertools
import json
import os
from pathlib import Path
import subprocess
import sys
import xml.etree.ElementTree as ET

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

REPO = Path(__file__).resolve().parents[1]
SEEDS = REPO / "tools/fixtures/seeds/render-markdown"
FIXTURES = "crates/d2-rs-render/tests/fixtures/"


def expand(seed: str) -> list[dict]:
    """Request list of one seed file, in capture order."""
    out = []
    for seg in json.loads((SEEDS / f"{seed}.json").read_text())["segments"]:
        if "axes" in seg:
            combos = []
            for pick in itertools.product(*[range(len(a["values"])) for a in seg["axes"]]):
                fields, labels = {}, []
                for axis, k in zip(seg["axes"], pick):
                    fields.update(zip(axis["fields"], axis["values"][k]))
                    labels.append(axis["labels"][k])
                combos.append({"label": "-".join(labels), "set": fields})
        else:
            combos = seg["combos"]
        for index, markdown in seg["snippets"]:
            for combo in combos:
                out.append({"Name": f"{seg['prefix']}-{index}-{combo['label']}", "Markdown": markdown,
                            **seg["const"], **combo["set"]})
    return out


def headings() -> list[dict]:
    seed = json.loads((SEEDS / "headings.json").read_text())
    sources = [f"{'#' * level} {seed['heading']}\n\n{body}" for level in seed["levels"] for body in seed["bodies"]]
    out = []
    for family in seed["families"]:
        for size in seed["sizes"]:
            for source in sources:
                level = len(source.split(" ", 1)[0])
                # the trailing number counts every case of the run
                out.append({"name": f"{family}-{size}-h{level}-{len(out)}",
                            "source": source, "family": family, "size": size})
    return out


TEXTMEASURE_JOBS = [
    # (output, hook kind, SVG role paint, requests)
    ("markdown-natural-source-v090.json", "natural", "none", lambda: expand("natural")),
    ("markdown-recursive-paint-source-v090.json", "cells", "full8", lambda: expand("blocks")),
    ("markdown-table-paint-source-v090.json", "cells", "table6", lambda: expand("tables")),
    ("markdown-mixed-flow-source-v090.json", "cells", "full8", lambda: expand("mixed")),
    ("markdown-table-inline-source-v090.json", "table-inline", "inline3", lambda: expand("table-inline")),
    ("paragraph-wrap-source-v090.json", "wrap", "inline3", lambda: expand("wrap")),
    ("markdown-heading-flow-source-v090.json", "heading", "none", headings),
]


def cmd_requests():
    jobs = [{"output": FIXTURES + out, "kind": kind, "paint": paint, "requests": make()}
            for out, kind, paint, make in TEXTMEASURE_JOBS]
    Path(os.environ["D2RS_RECIPE_OUTPUT"]).write_text(json.dumps(jobs, ensure_ascii=False) + "\n")


def go_test_hook(package: str, hook: str, test: str, env: dict[str, str]):
    """`go test` one hook overlaid into the runner's patched upstream tree."""
    upstream = Path(os.environ["D2RS_UPSTREAM"])
    overlay = fixture_recipe.workdir() / "overlay.json"
    overlay.write_text(json.dumps({"Replace": {str(upstream / package / Path(hook).name): str(REPO / hook)}}))
    subprocess.run(["go", "test", "-mod=readonly", "-count=1", "-overlay", str(overlay),
                    "-run", f"^{test}$", f"./{package}"],
                   cwd=upstream, env={**fixture_recipe.go_env(), **env}, check=True)


def cmd_scene():
    work = fixture_recipe.workdir()
    requests, raw = work / "scene-requests.json", work / "scene-rows.json"
    requests.write_text(json.dumps(expand("scene"), ensure_ascii=False))
    go_test_hook("d2renderers/d2scenebuild", "tools/fixtures/hooks/render-markdown-scene/d2rs_markdown_scene_test.go",
                 "TestD2RSMarkdownScene", {"D2RS_SCENE_INPUT": str(requests), "D2RS_SCENE_OUTPUT": str(raw)})
    rows = json.loads(raw.read_text())
    # Complete rows for the one snippet whose viewport/clip contract is checked.
    viewport = [r for r in rows if r["request"]["Name"].startswith("metadata-21-")]
    fixture_recipe.output(FIXTURES + "markdown-layout-viewport-source-v090.json").write_text(
        json.dumps(viewport, indent=2) + "\n")
    metadata = []
    for s in rows:
        row = {"request": s["request"],
               "layout": {k: s["layout"][k] for k in ("Width", "Height", "Primitives")}}
        if "error" in s:
            row["error"] = s["error"]
        else:
            row["links"] = s["links"] or []
            titles: dict[int, list[str]] = {}
            for node in s["retained"]:
                parts = node["id"].split(":")
                if len(parts) == 5 and parts[:2] == ["appendix", "row"] and parts[3] == "text":
                    titles.setdefault(int(parts[2]), []).append(node["primitive"]["Text"])
            row["appendix_titles"] = ["\n".join(titles[k]) for k in sorted(titles)]
        metadata.append(row)
    fixture_recipe.output(FIXTURES + "markdown-scene-metadata-source-v090.json").write_text(
        json.dumps(metadata, separators=(",", ":"), ensure_ascii=False) + "\n")


def cmd_tooltip():
    work = fixture_recipe.workdir()
    raw, svgs = work / "tooltip-rows.json", work / "tooltip-svg"
    go_test_hook("d2renderers/d2svg", "tools/fixtures/hooks/render-positioned-tooltip/d2rs_positioned_tooltip_test.go",
                 "TestD2RSPositionedTooltip", {"D2RS_TOOLTIP_OUTPUT": str(raw), "D2RS_TOOLTIP_SVG_DIR": str(svgs)})
    rows = json.loads(raw.read_text())
    tag = lambda el: el.tag.rsplit("}", 1)[-1]
    for row in rows:
        if "svg_file" not in row:
            continue
        root = ET.fromstring((svgs / row["svg_file"]).read_bytes())
        row["svg_viewboxes"] = [el.get("viewBox") for el in root.iter() if tag(el) == "svg" and el.get("viewBox") is not None]
        callouts = []
        for group in root.iter():
            if group.get("class") != "positioned-tooltip":
                continue
            rect = next(el for el in group if tag(el) == "rect")
            tail = next(el for el in group if tag(el) == "path")
            callouts.append({
                "box": [float(rect.get(k)) for k in ("x", "y", "width", "height", "rx", "ry")],
                "tail": tail.get("d"),
                "text": ["".join(el.itertext()) for el in group.iter() if tag(el) == "text"],
                "viewports": [dict(el.attrib) for el in group.iter() if tag(el) == "svg"],
            })
        row["callouts"] = callouts
    fixture_recipe.output(FIXTURES + "positioned-tooltip-source-v090.json").write_text(json.dumps(rows, indent=2) + "\n")


if __name__ == "__main__":
    {"requests": cmd_requests, "scene": cmd_scene, "tooltip": cmd_tooltip}[sys.argv[1]]()
