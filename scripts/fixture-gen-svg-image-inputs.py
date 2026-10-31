"""Input generator for the mathtext-img SVG image recipes (tools/fixtures/mathtext-img.toml).

usage: fixture-gen-svg-image-inputs.py {assets|import|asset-cache|document-import}

Writes $D2RS_RECIPE_OUTPUT: a JSON list of strings. `assets` is the list of
<image> href values (valid 2x3 PNG/GIF/JPEG/WebP data: URIs, truncated and
malformed ones, a remote URL, a MIME/signature mismatch). `import` places each
href in ten document templates (plain, repeated, hidden, transparent, used,
clipped, nested viewport, before/after an invalid path) plus geometry and
paint edge cases. The scene-build inputs reuse the first eight PNG documents
plus a few vector-only documents.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

PNG = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAYAAAC56t6BAAAAAElEQVQ1rwYeAAAAAElFTkSuQmCC"
HREFS = [
    "data:image/png;base64," + PNG,
    "data:image/gif;base64,R0lGODlhAgADAAAAACwAAAAAAAAAAAACADs=",
    "data:image/jpeg;base64,/9j/wAALCAADAAIBAREA/9oAAv/Z",
    "data:image/webp;base64,UklGRhIAAABXRUJQVlA4TAUAAAAvAYAAAAA=",
    "data:image/png;base64,iVBORw0KGgo=",  # signature only
    "data:image/gif;base64,R0lGODlhAgADAAAAADs=",  # no image block
    "data:image/jpeg;base64,/9j/2Q==",  # SOI + EOI
    "data:image/webp;base64,UklGRhIAAABXRUJQVlA4TAUAAAAAAAAAAAA=",  # bad VP8L signature
    "https://secret/x",
    "data:image/png;base64,AB==",  # non-canonical base64
    "data:image/jpeg;base64," + PNG,  # MIME/signature mismatch
]

ROOT = "<svg id='root' width='10' height='20'>"


def image(href: str, extra: str = "") -> str:
    return f"<image href='{href}' width='2' height='3'{extra}/>"


def templates(href: str) -> list[str]:
    img = image(href)
    return [
        ROOT + img + "</svg>",
        ROOT + img + img + "</svg>",
        ROOT + "<g display='none'>" + img + "</g></svg>",
        ROOT + "<g visibility='hidden'>" + img + "</g></svg>",
        ROOT + "<g opacity='0' fill='none' stroke='red'>" + img + "</g></svg>",
        ROOT + "<defs>" + img.replace("<image ", "<image id=\"p\" ") + "</defs><use href='#p'/><use href='#p' x='5'/></svg>",
        ROOT + "<clipPath id='c'><rect width='2' height='2'/></clipPath><g clip-path='url(#c)'>" + img + "</g></svg>",
        ROOT + "<svg x='2' y='3' width='5' height='6' viewBox='0 0 10 12'>" + img + "</svg></svg>",
        ROOT + img + "<path d='bad'/></svg>",
        ROOT + "<path d='bad'/>" + img + "</svg>",
    ]


def png_edge_cases() -> list[str]:
    href = HREFS[0]
    head = "<svg width='10' height='20'>"
    return [head + f"<image href='{href}' {body}/></svg>" for body in (
        "",
        "width='0' height='3'",
        "width='2' height='3' preserveAspectRatio='none'",
        "width='2' height='3' preserveAspectRatio='xMaxYMax slice'",
        "x='1e308' width='1e308' height='3'",
        "width='2' height='3' transform='scale(1e308)'",
        "width='2' height='3' fill-opacity='0' visibility='visible'",
    )]


def main() -> None:
    kind = sys.argv[1]
    png_docs = templates(HREFS[0])[:8]
    rect = "<svg width='10' height='20'><rect width='2' height='3'/></svg>"
    if kind == "assets":
        values = HREFS
    elif kind == "import":
        values = [doc for href in HREFS for doc in templates(href)] + png_edge_cases()
    elif kind == "asset-cache":
        values = png_docs + [rect, "<svg viewBox='1 2 10 20'><path d='M0 0L1 2'/></svg>"]
    elif kind == "document-import":
        values = png_docs + [rect, "<svg width='10' height='20'><path d='bad'/></svg>",
                             "<svg width='10' height='20'><defs><path id='p' d='M0 0L2 3'/></defs><use href='#p'/><use href='#p'/></svg>"]
    else:
        sys.exit(f"unknown input kind {kind!r}")
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(json.dumps(values, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
