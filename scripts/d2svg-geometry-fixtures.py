#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Regenerate the d2svg geometry fixtures under
crates/d2-rs-render/tests/fixtures/d2svg-geometry/.

golden.json  -- shape/connection geometry from every non-sketch D2 v0.9.0
                e2e golden board (e2etests/testdata/**/board.exp.json) paired
                with the `d="..."`, `<text x y>` and `<image x y>` values the
                pinned Go renderer wrote into the sibling sketch.exp.svg.
oracle.json  -- bit-exact outputs of the pinned Go lib/geo, lib/label,
                lib/shape, lib/svg and math packages for fixed-seed inputs
                (scripts/d2svg-geometry-oracle.go, built inside ../d2 with
                `go build -overlay` so the pristine checkout is not modified).

Run through the fixture runner (recipes d2svg-geometry-golden and
d2svg-geometry-oracle in tools/fixtures/renderexport-isometric.toml):
    uv run --no-project python scripts/regen-fixtures.py --only 'd2svg-geometry-*'
The outputs go to the recipe's staged paths (oracle-cache/fixtures/...), never
into the repository. Both read the runner's pinned upstream tree; oracle builds inside the runner's upstream tree ($D2RS_UPSTREAM).
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import fixture_recipe  # noqa: E402
import references  # noqa: E402

OUT = "crates/d2-rs-render/tests/fixtures/d2svg-geometry"


def go_escape_text(s: str) -> str:
    """encoding/xml.EscapeText, as lib/svg.EscapeText uses it."""
    out = []
    for ch in s:
        o = ord(ch)
        if ch == '"':
            out.append("&#34;")
        elif ch == "'":
            out.append("&#39;")
        elif ch == "&":
            out.append("&amp;")
        elif ch == "<":
            out.append("&lt;")
        elif ch == ">":
            out.append("&gt;")
        elif ch == "\t":
            out.append("&#x9;")
        elif ch == "\n":
            out.append("&#xA;")
        elif ch == "\r":
            out.append("&#xD;")
        elif not (o in (0x9, 0xA, 0xD) or 0x20 <= o <= 0xD7FF or 0xE000 <= o <= 0xFFFD or 0x10000 <= o <= 0x10FFFF):
            out.append("�")
        else:
            out.append(ch)
    return "".join(out)


def group_class(id_: str) -> str:
    return base64.urlsafe_b64encode(go_escape_text(id_).encode()).decode()


G_TOKEN = re.compile(r"<g[\s>]|</g>")


def group_body(svg: str, start: int) -> str:
    """The text of the <g> element opened at `start`, with nesting."""
    depth = 0
    for m in G_TOKEN.finditer(svg, start):
        if m.group(0) == "</g>":
            depth -= 1
            if depth == 0:
                return svg[start : m.end()]
        else:
            depth += 1
    return svg[start:]


D_ATTR = re.compile(r' d="([^"]*)"')
TEXT_XY = re.compile(r'<text x="([^"]*)" y="([^"]*)"')
IMAGE_XY = re.compile(r'<image href="[^"]*" x="([^"]*)" y="([^"]*)" width="(\d+)" height="(\d+)"')


def find_group(svg: str, cls: str, from_pos: int = 0) -> str | None:
    m = re.compile(r'<g class="' + re.escape(cls) + r'[" ]').search(svg, from_pos)
    if not m:
        return None
    return group_body(svg, m.start())


# DSL types d2svg renders without lib/shape path data.
PRIMITIVE_TYPES = {"", "rectangle", "square", "oval", "circle", "text", "code", "class", "sql_table", "image", "sequence_diagram", "hierarchy"}


def compact(rec: dict) -> dict:
    """Drop default-valued keys to keep the fixture small (positions stay)."""

    def is_default(k, v):
        if k in ("x", "y"):
            return False
        if k == "op":
            return v == 1 and not isinstance(v, bool)
        if isinstance(v, bool):
            return v is False
        return v is None or v == "" or v == [] or (isinstance(v, (int, float)) and v == 0)

    return {k: v for k, v in rec.items() if not is_default(k, v)}


def shape_record(s: dict, body: str | None) -> dict:
    # Short keys: t=type, w/h=size, m=multiple, td=3d, db=double-border,
    # l=has label, lang=language, lp=label position, lw/lh=label size,
    # fs=font size, op=opacity, ic=has icon, ip=icon position,
    # ar=content aspect ratio, nf=group not found in the SVG,
    # d=path data, tx=<text x y>, im=<image x y w h>.
    rec = {
        "id": s["id"],
        "t": s["type"],
        "x": s["pos"]["x"],
        "y": s["pos"]["y"],
        "w": s["width"],
        "h": s["height"],
        "m": s.get("multiple", False),
        "td": s.get("3d", False),
        "db": s.get("double-border", False),
        "l": s.get("label", "") != "",
        "lang": s.get("language", ""),
        "lp": s.get("labelPosition", ""),
        "lw": s.get("labelWidth", 0),
        "lh": s.get("labelHeight", 0),
        "fs": s.get("fontSize", 0),
        "op": s.get("opacity", 1),
        "ic": s.get("icon") is not None,
        "ip": s.get("iconPosition", ""),
        "ar": s.get("contentAspectRatio"),
        "nf": body is None,
    }
    if body is not None:
        # Path data only for shapes d2svg draws from lib/shape paths; text
        # anchors only for plain-text labels; images only with an icon.
        if s["type"] not in PRIMITIVE_TYPES:
            rec["d"] = D_ATTR.findall(body)
        if rec["l"] and not rec["lang"]:
            rec["tx"] = [list(t) for t in TEXT_XY.findall(body)][:1]
        if rec["ic"]:
            rec["im"] = [list(t) for t in IMAGE_XY.findall(body)]
    return compact(rec)


def connection_record(c: dict, body: str | None) -> dict:
    # r=route, sw=stroke width, lpc=label percentage; other keys as above.
    rec = {
        "id": c["id"],
        "r": [[p["x"], p["y"]] for p in (c.get("route") or [])],
        "sw": c.get("strokeWidth", 0),
        "l": c.get("label", "") != "",
        "lang": c.get("language", ""),
        "lp": c.get("labelPosition", ""),
        "lpc": c.get("labelPercentage", 0),
        "lw": c.get("labelWidth", 0),
        "lh": c.get("labelHeight", 0),
        "fs": c.get("fontSize", 0),
        "nf": body is None,
    }
    if body is not None and not rec["lang"]:
        rec["tx"] = [list(t) for t in TEXT_XY.findall(body)][:1]
    return compact(rec)


def golden() -> dict:
    testdata = Path(os.environ["D2RS_UPSTREAM"]).resolve() / "e2etests/testdata"
    boards = []
    skipped = {"sketch": 0, "no_svg": 0, "nothing_to_check": 0}
    for board_path in sorted(testdata.rglob("board.exp.json")):
        svg_path = board_path.with_name("sketch.exp.svg")
        if not svg_path.exists():
            skipped["no_svg"] += 1
            continue
        board = json.loads(board_path.read_text())
        if (board.get("config") or {}).get("sketch"):
            skipped["sketch"] += 1
            continue
        svg = svg_path.read_text()
        shapes = []
        for s in board.get("shapes") or []:
            rec = shape_record(s, find_group(svg, group_class(s["id"])))
            if not any(k in rec for k in ("d", "tx", "im", "nf")):
                skipped["nothing_to_check"] += 1
                continue
            shapes.append(rec)
        conns = []
        for c in board.get("connections") or []:
            if not c.get("label"):
                continue  # only labelled connections carry checked geometry
            conns.append(connection_record(c, find_group(svg, group_class(c["id"]))))
        boards.append(
            {
                "board": str(board_path.parent.relative_to(testdata)),
                "shapes": shapes,
                "connections": conns,
            }
        )
    return {"source": f"d2 {references.SOURCE_COMMIT[:9]} e2etests/testdata", "skipped": skipped, "boards": boards}


def oracle() -> bytes:
    upstream = Path(os.environ["D2RS_UPSTREAM"]).resolve()
    env = fixture_recipe.go_env()
    tmp = fixture_recipe.workdir()
    overlay = tmp / "overlay.json"
    overlay.write_text(
        json.dumps(
            {"Replace": {str(upstream / "cmd/d2svggeooracle/main.go"): str(ROOT / "scripts/d2svg-geometry-oracle.go")}}
        )
    )
    binary = tmp / "oracle"
    subprocess.run(
        ["go", "build", "-mod=readonly", "-tags", "d2svgoracle", "-overlay", str(overlay), "-o", str(binary), "./cmd/d2svggeooracle"],
        cwd=upstream,
        env=env,
        check=True,
    )
    return subprocess.run([str(binary)], check=True, capture_output=True).stdout


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--golden", action="store_true")
    ap.add_argument("--oracle", action="store_true")
    args = ap.parse_args()
    both = not (args.golden or args.oracle)
    if both or args.golden:
        data = golden()
        fixture_recipe.output(OUT + "/golden.json").write_text(json.dumps(data, separators=(",", ":"), sort_keys=True) + "\n")
        n = sum(len(b["shapes"]) for b in data["boards"])
        print(f"golden.json: {len(data['boards'])} boards, {n} shapes, skipped {data['skipped']}")
    if both or args.oracle:
        raw = json.loads(oracle())
        fixture_recipe.output(OUT + "/oracle.json").write_text(json.dumps(raw, separators=(",", ":"), sort_keys=True) + "\n")
        print("oracle.json:", {k: len(v) for k, v in raw.items()})
    return 0


if __name__ == "__main__":
    sys.exit(main())
