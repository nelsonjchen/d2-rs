#!/usr/bin/env python3
"""Request generators for the renderexport-export oracle recipes.

    python scripts/renderexport_inputs.py <kind> [output]

`kind` is codec or raster-validation. The output defaults to
$D2RS_RECIPE_OUTPUT (the recipe's generated input path;
tools/fixtures/renderexport-export.toml). Every list is a deterministic grid;
compressed or encoded bytes that cannot be produced with
the Python standard library come from the small seeds under
tools/fixtures/inputs/renderexport/.
"""
from __future__ import annotations

import base64
import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
SEEDS = ROOT / "tools/fixtures/inputs/renderexport"


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
