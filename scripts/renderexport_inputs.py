#!/usr/bin/env python3
"""Request generators for the renderexport-export oracle recipes.

    python scripts/renderexport_inputs.py <kind> [output]

`kind` is one of gradient, svg-transforms, svg-anonymous, codec,
raster-validation. The output defaults to $D2RS_RECIPE_OUTPUT (the recipe's
generated input path; tools/fixtures/renderexport-export.toml). Every list is
a deterministic grid; compressed or encoded bytes that cannot be produced with
the Python standard library come from the small seeds under
tools/fixtures/inputs/renderexport/.
"""
from __future__ import annotations

import base64
import itertools
import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
SEEDS = ROOT / "tools/fixtures/inputs/renderexport"


def num(v) -> str:
    """Python str() of the authored grid value (ints stay ints)."""
    return str(v)


# --------------------------------------------------------------------------
# d2raster prepared gradient sampling (gradient-samples-source-v090.json)

GRADIENT_MATRICES = [[1, 0, 0, 1, 8, 4], [1.5, 0, 0, 0.5, 12, 2], [1, 0.25, 0.5, 1, 10, 8], [0, 1, -1, 0, 32, 8]]
GRADIENT_TRANSFORMS = [[1, 0, 0, 1, 0, 0], [1, 0.125, 0.25, 1, 0.0625, -0.125]]
GRADIENT_STOPS = [
    [[0, 255, 255, 255, 255], [1, 0, 0, 0, 255]],
    [[0, 255, 0, 0, 64], [0.375, 0, 255, 0, 128], [0.375, 0, 0, 255, 191], [1, 255, 255, 255, 255]],
]
GRADIENT_GEOMETRY = {
    "linear": [[0, 0, 0, 1], [0, 0, 1, 1], [1, 0.25, 0.125, 0.75]],
    "radial": [[0.5, 0.5, 0, 0.5, 0.5, 0.5], [0.25, 0.5, 0, 0.5, 0.5, 0.5], [0.5, 0.5, 0.125, 0.5, 0.5, 0.5]],
}
SPREADS = ["pad", "reflect", "repeat"]


def gradient():
    cases = []
    for kind in ("linear", "radial"):
        for geometry, matrix, transform, spread, stops in itertools.product(
                GRADIENT_GEOMETRY[kind], GRADIENT_MATRICES, GRADIENT_TRANSFORMS, range(3), GRADIENT_STOPS):
            if kind == "linear":
                names = ("x1", "y1", "x2", "y2")
            else:
                names = ("fx", "fy", "fr", "cx", "cy", "r")
            attrs = " ".join(f'{n}="{num(v)}"' for n, v in zip(names, geometry))
            stop_xml = "".join(
                f'<stop offset="{num(s[0])}" stop-color="rgb({s[1]},{s[2]},{s[3]})" stop-opacity="{s[4] / 255}"/>'
                for s in stops)
            element = f"{kind}Gradient"
            svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="48" height="32"><defs>'
                   f'<{element} id="g" {attrs} gradientTransform="matrix({" ".join(map(num, transform))})" '
                   f'spreadMethod="{SPREADS[spread]}">{stop_xml}</{element}></defs>'
                   f'<g transform="matrix({" ".join(map(num, matrix))})">'
                   '<rect x="-8" y="-4" width="48" height="32" fill="url(#g)"/></g></svg>')
            cases.append({"name": f"{kind}-{len(cases)}", "svg": svg, "kind": kind, "geometry": geometry,
                          "matrix": matrix, "gradient_matrix": transform, "spread": spread, "stops": stops})
    return cases


# --------------------------------------------------------------------------
# d2svgimport public import + vector-image placement grids
# (svg-image-transforms / svg-image-anonymous-paths-source-v090.json)

SVG_ASPECTS = ["xMidYMid meet", "none", "xMaxYMax slice"]
REQUEST_ASPECTS = ["xMidYMid meet", "none", "xMinYMin meet", "xMaxYMax slice"]
BOXES = [[12.125, 23.75, 46, 46], [1814, 455, 46, 46], [0.3333333333333, -0.6666666666667, 92.25, 61.125]]
CANVAS = [-101.125, -50.75, 2580, 918]
DIMENSIONS = {
    "viewbox": 'viewBox="3.125 -7.75 70.28 68.2"',
    "dimensions": 'width="140.56" height="136.4" viewBox="3.125 -7.75 70.28 68.2"',
    "onewidth": 'width="140.56" viewBox="3.125 -7.75 70.28 68.2"',
    "oneheight": 'height="136.4" viewBox="3.125 -7.75 70.28 68.2"',
    "units": 'width="140.56px" height="136.4px" viewBox="3.125 -7.75 70.28 68.2"',
    "rect": 'width="70.28" height="68.2"',
}
INK = ".ink{fill:#326ce5;stroke:white;stroke-width:.25"
ANONYMOUS = {
    # variant: (extra .ink declarations, outer <g> attributes, extra ink path attributes)
    "paint": ("", "", ""),
    "alpha": (";fill-opacity:.5;stroke-opacity:.75", "", ""),
    "hidden": (";visibility:hidden", "", ""),
    "below": (";fill-opacity:.49999999;stroke-opacity:.74999999", "", ""),
    "above": (";fill-opacity:.50000001;stroke-opacity:.75000001", "", ""),
    "inherit": ("", ' fill-opacity=".49999999" stroke-opacity=".75000001"', ""),
    "override": (";fill-opacity:.5;stroke-opacity:.75000001", "",
                 ' fill-opacity=".9" stroke-opacity=".3" style="fill-opacity:.49999999;stroke-opacity:inherit"'),
}


def svg_image(dims: str, aspect: str, variant: str | None) -> str:
    declarations, group, path_attrs = ANONYMOUS[variant] if variant else ("", "", "")
    style = INK + declarations + "}" + (".blank{fill:none;stroke:none}" if variant else "")
    ink_id = "" if variant else ' id="ink"'
    blank = '<path class="blank" d="M1 1h10v10z"/>' if variant else ""
    return ('<svg xmlns="http://www.w3.org/2000/svg" xmlns:editor="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" '
            f'{DIMENSIONS[dims]} preserveAspectRatio="{aspect}" style="enable-background:new 3.125 -7.75 70.28 68.2;">'
            f'<style>{style}</style><editor:namedview/><g{group} transform="translate(-6.3260942,-17.47524)">'
            '<g transform="matrix(.95 .125 -.0625 1.2 3.3333333333333 -4.4444444444444)">'
            f'<path{ink_id} class="ink"{path_attrs} transform="translate(.123456789 -.234567891)" d="M3.125 -7.75h50v50h-50z"/>'
            f'{blank}</g></g></svg>')


def svg_requests(variants):
    requests = []
    for dims, svg_aspect, box, aspect, scale in itertools.product(
            DIMENSIONS, SVG_ASPECTS, range(len(BOXES)), REQUEST_ASPECTS, (1, 2, 3)):
        for variant in variants:
            prefix = f"anonymous-{variant}-" if variant else ""
            requests.append({"Name": f"{prefix}{dims}-{svg_aspect}-{box}-{aspect}-{scale}",
                             "SVG": svg_image(dims, svg_aspect, variant), "Aspect": aspect,
                             "Box": BOXES[box], "Canvas": CANVAS, "Scale": scale})
    return requests


# --------------------------------------------------------------------------
# lib/imageasset probeResource codec outcomes (svg-asset-codec-outcomes-source-v090.json)


def codec():
    seed = json.loads((SEEDS / "svg-asset-codec-seed.json").read_text())
    large = ('<svg width="16" height="16">' + " " * seed["large_padding"] + "</svg>").encode()
    rows = []
    for row in seed["rows"]:
        row = dict(row)
        if row.pop("data_large", False):
            row["data_base64"] = base64.b64encode(large).decode()
        rows.append({"name": row["name"], "data_base64": row["data_base64"],
                     "declared": row["declared"], "limit": row["limit"]})
    for byte in range(256):
        rows.append({"name": f"one-byte-{byte:02x}", "data_base64": base64.b64encode(bytes([byte])).decode(),
                     "declared": True, "limit": 4194304})
    return rows


# --------------------------------------------------------------------------
# d2raster raster asset validation/preparation (raster-validation-v090.json)


def raster_validation():
    seed = json.loads((SEEDS / "raster-validation-seed.json").read_text())
    images = [list(base64.b64decode(i)) for i in seed["images"]]
    fields = seed["fields"]
    rows = []
    for case in seed["cases"]:
        row = dict(zip(fields, case))
        row["Data"] = images[row["Data"]]
        rows.append({"ID": seed["id"], **{k: row[k] for k in
                     ("MIME", "Data", "Width", "Height", "DecodedBytes", "Available", "Mode", "Cancel")}})
    return rows


KINDS = {
    "gradient": gradient,
    "svg-transforms": lambda: svg_requests([None]),
    "svg-anonymous": lambda: svg_requests(list(ANONYMOUS)),
    "codec": codec,
    "raster-validation": raster_validation,
}


def main():
    if len(sys.argv) not in (2, 3) or sys.argv[1] not in KINDS:
        sys.exit(f"usage: {sys.argv[0]} {{{','.join(KINDS)}}} [output]")
    output = sys.argv[2] if len(sys.argv) == 3 else os.environ.get("D2RS_RECIPE_OUTPUT")
    if not output:
        sys.exit("no output: pass a path or run through scripts/regen-fixtures.py")
    Path(output).write_text(json.dumps(KINDS[sys.argv[1]](), indent=1) + "\n")


if __name__ == "__main__":
    main()
