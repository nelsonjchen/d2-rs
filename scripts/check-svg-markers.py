#!/usr/bin/env -S uv run python
# SPDX-License-Identifier: MPL-2.0
"""Compare referenced SVG markers; opaque resource IDs may differ.

This is a component check, not a full SVG or rendered-pixel parity gate.
Only marker root IDs and XML attribute ordering/formatting whitespace are
normalized. Marker geometry, dimensions, styles, occurrence order and endpoint
attachment remain exact. Missing or duplicate referenced definitions fail.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import xml.etree.ElementTree as ET


def canonical(element, marker_root=False):
    return {
        "tag": element.tag,
        "attributes": dict(sorted((key, value) for key, value in element.attrib.items()
                                  if not (marker_root and key == "id"))),
        "text": (element.text or "").strip(),
        "children": [canonical(child) for child in element],
    }


def referenced_markers(path):
    document = ET.fromstring(path.read_bytes())
    definitions = {}
    for element in document.iter():
        if element.tag.rsplit("}", 1)[-1] != "marker":
            continue
        identity = element.attrib["id"]
        if identity in definitions:
            raise ValueError(f"duplicate marker {identity} in {path}")
        definitions[identity] = canonical(element, True)
    references = []
    for element in document.iter():
        endpoints = {}
        for name in ("marker-start", "marker-mid", "marker-end"):
            if name not in element.attrib:
                continue
            match = re.fullmatch(r"url\(#([^\s)]+)\)", element.attrib[name])
            if match is None or match[1] not in definitions:
                raise ValueError(f"missing or unsupported {name} in {path}")
            endpoints[name] = definitions[match[1]]
        if endpoints:
            references.append({"element": element.tag, "endpoints": endpoints})
    if not references:
        raise ValueError(f"no marker references in {path}")
    return references


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go", type=Path, required=True)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    captures = {}
    markers = {}
    for side in ("go", "rust"):
        path = getattr(args, side)
        captures[side] = {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        markers[side] = referenced_markers(path)
    passed = markers["go"] == markers["rust"]
    result = {"passed": passed, "scope": __doc__, "captures": captures,
              "referenced_markers": markers, "full_render_parity": False}
    args.report.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"passed": passed, "report": str(args.report),
                      "counts": {side: len(rows) for side, rows in markers.items()}}))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
