#!/usr/bin/env python3
"""Extract d2-rs-export PDF fixtures from Go-written FPDF documents (stdlib only).

    python scripts/export_pdf_fixtures.py navigation|fonts

Reads the intermediate PDFs of the `renderexport-public-pdf-*-docs` recipes
from $D2RS_FIXTURES_DIR (tools/renderexport-pdf-{navigation,fonts}/) and writes
the recipe output (tools/fixtures/renderexport-export-public.toml):

- navigation: per document, each page's MediaBox size and its link
  annotations (/Rect, /Dest page index, /XYZ left/top, null zoom).
  Replaces the pypdf analysis of .reference/codex-runs run32.
- fonts: per document and font (regular/bold), the text drawn, the Type0
  font's first object number, the seven raw FPDF font objects and the
  decoded FontFile2 subset. Replaces witness-inputs/run36-extract-fonts.py.
"""
from __future__ import annotations

import base64
import json
import os
from pathlib import Path
import re
import sys
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

OBJECT = re.compile(rb"(?m)^(\d+) 0 obj\n")


def objects(data: bytes) -> dict[int, tuple[int, int]]:
    """Object number -> (start, end) byte span, end = next object or xref."""
    starts = [(int(m[1]), m.start()) for m in OBJECT.finditer(data)]
    xref = data.rindex(b"\nxref\n") + 1
    bounds = sorted(s for _, s in starts) + [xref]
    following = {s: bounds[i + 1] for i, s in enumerate(bounds[:-1])}
    return {n: (s, following[s]) for n, s in starts}


def body(data: bytes, spans, number: int) -> bytes:
    start, end = spans[number]
    return data[start:end]


def navigation(fixtures: Path) -> list:
    rows = []
    for name, nav in (("plain", False), ("nav", True)):
        data = (fixtures / "tools/renderexport-pdf-navigation" / f"{name}.pdf").read_bytes()
        spans = objects(data)
        pages_obj = next(n for n in spans if b"/Type /Pages" in body(data, spans, n))
        kids = [int(k) for k in re.findall(rb"(\d+) 0 R", re.search(rb"/Kids \[([^\]]*)\]", body(data, spans, pages_obj))[1])]
        pages = []
        for kid in kids:
            page = body(data, spans, kid)
            box = [float(v) for v in re.search(rb"/MediaBox \[([^\]]*)\]", page)[1].split()]
            links = []
            for rect, dest in re.findall(rb"/Subtype /Link /Rect \[([^\]]*)\][^>]*?/Dest \[([^\]]*)\]", page):
                target, _, _, kind, left, top, zoom = dest.split()
                assert zoom == b"null", dest
                links.append({"rect": [float(v) for v in rect.split()], "destination_page": kids.index(int(target)),
                              "destination_kind": kind.decode(), "destination_left": float(left),
                              "destination_top": float(top), "preserve_zoom": True})
            pages.append({"size": box[2:], "links": links})
        rows.append({"navigation": nav, "pages": pages})
    return rows


def fonts(fixtures: Path) -> list:
    rows = []
    for name, regular, bold in (("empty", "", ""), ("board", "blah  /  ", "blahdog"), ("unicode", "Café (α) \\", "Résumé 東京")):
        data = (fixtures / "tools/renderexport-pdf-fonts" / f"{name}.pdf").read_bytes()
        spans = objects(data)
        row = {"name": name}
        for first in sorted(n for n in spans if b"/Subtype /Type0" in body(data, spans, n)):
            font = body(data, spans, first)
            key = "bold" if b"/BaseFont /utf8sourceB\n" in font else "regular"
            assert b"/BaseFont /utf8source" in font
            raw = data[spans[first][0]:spans[first + 6][1]]
            assert raw.endswith(b"endobj\n")
            descriptor = next(body(data, spans, n) for n in range(first, first + 7) if b"/FontFile2" in body(data, spans, n))
            file2 = int(re.search(rb"/FontFile2 (\d+) 0 R", descriptor)[1])
            stream = body(data, spans, file2)
            length = int(re.search(rb"/Length (\d+)", stream)[1])
            payload = stream[stream.index(b"stream\n") + 7:][:length]
            row[key] = {"text": bold if key == "bold" else regular, "first_object": first,
                        "objects": base64.b64encode(raw).decode(),
                        "subset": base64.b64encode(zlib.decompress(payload)).decode()}
        assert set(row) == {"name", "regular", "bold"}, row.keys()
        rows.append(row)
    return rows


def main() -> None:
    fixtures = Path(os.environ["D2RS_FIXTURES_DIR"])
    kind = sys.argv[1]
    if kind == "navigation":
        value, logical = navigation(fixtures), "crates/d2-rs-export/tests/fixtures/pdf-navigation-source-v090.json"
    elif kind == "fonts":
        value, logical = fonts(fixtures), "crates/d2-rs-export/tests/fixtures/pdf-fonts-fpdf.json"
    else:
        sys.exit(f"unknown kind {kind}")
    fixture_recipe.output(logical).write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
