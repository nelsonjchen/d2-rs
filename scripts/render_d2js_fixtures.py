#!/usr/bin/env python3
"""Public d2js raw-target render fixtures for crates/d2-rs-render (recipe
`render-d2js-public-target`, after `d2js-node-package`).

Runs tools/d2js-capture/render-public-target.mjs against the pinned d2js
package and writes public-target-v090.json (eight inputs x four target
mutations, compiled with dagre and rendered with pad 20 / noXMLTag) and
public-target-precision-v090.json (fractional route coordinates). The hash
class, viewBox and SVG sha256 are read from the Go SVG as the v0.9.0-era
captures recorded them.

    uv run --no-project python scripts/regen-fixtures.py --only render-d2js-public-target
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
FIXTURES = "crates/d2-rs-render/tests/fixtures/"


def sha(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


def main_hash(svg: str) -> str:
    return re.search(r'class="(d2-\d+)', svg).group(1)


def main() -> None:
    commit = json.loads((ROOT / "reference-manifest.json").read_text())["d2"]["commit"]
    raw_path = fixture_recipe.workdir() / "public-target-observations.json"
    subprocess.run(["node", str(ROOT / "tools/d2js-capture/render-public-target.mjs"), str(raw_path),
                    str(ROOT / "tools/d2js-capture/inputs")], check=True)
    raw = json.loads(raw_path.read_text())
    cases = []
    for row in raw["target"]:
        assert "svg" in row, (row["name"], row.get("error"))
        svg = row["svg"]
        cases.append({"name": row["name"], "source": row["input"], "diagram": row["diagram"], "render_options": row["options"],
                      "expected_hash": main_hash(svg), "expected_view_box": ET.fromstring(svg).get("viewBox"),
                      "source_svg_sha256": sha(svg)})
    provenance = (f"Pinned Go D2 {commit} d2js; compile with dagre then JSON round-trip/modify diagram and render "
                  "with pad20/noXMLTag.")
    fixture_recipe.output(FIXTURES + "public-target-v090.json").write_text(json.dumps(
        {"provenance": provenance, "capture_script": "tools/d2js-capture/render-public-target.mjs", "cases": cases},
        indent=2, ensure_ascii=False) + "\n")
    precision = []
    for row in raw["precision"]:
        svg = row["svg"]
        precision.append({"name": row["name"], "input": row["input"], "diagram": row["diagram"], "options": row["options"],
                          "output": f"source-{row['name']}.svg", "sha256": sha(svg),
                          "expected_coordinate": row["expected_coordinate"],
                          "expected_view_box": ET.fromstring(svg).get("viewBox"), "expected_hash": main_hash(svg)})
    fixture_recipe.output(FIXTURES + "public-target-precision-v090.json").write_text(
        json.dumps(precision, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
