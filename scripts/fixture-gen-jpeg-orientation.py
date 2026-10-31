"""Input generator for recipe mathtext-img-jpeg-orientation (tools/fixtures/mathtext-img.toml).

Writes $D2RS_RECIPE_OUTPUT: a JSON list of byte arrays for
internal/rasterimage.jpegEXIFOrientation. Inputs are JPEG prefixes carrying an
APP1 Exif segment whose IFD0 holds two Orientation (0x0112) entries: the first
varies value 0..10 and (type, count) over SHORT/1, ASCII/1 and SHORT/2, the
second is a valid SHORT orientation 6. Each such seed contributes every prefix
plus itself again. Per byte order (II, MM) two more inputs probe a 258-entry
IFD with the orientation last, and duplicate APP1 segments.
"""
from __future__ import annotations

import json
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

SOI, SOS = b"\xff\xd8", b"\xff\xda"


def entry(order: str, tag: int, typ: int, count: int, value: int) -> bytes:
    # The value is stored left-justified as a SHORT whatever the declared type.
    return struct.pack(order + "HHIH", tag, typ, count, value) + b"\x00\x00"


def app1(order: str, entries: list[bytes]) -> bytes:
    tiff = (b"II*\x00" if order == "<" else b"MM\x00*") + struct.pack(order + "I", 8)
    body = b"Exif\x00\x00" + tiff + struct.pack(order + "H", len(entries)) + b"".join(entries)
    return b"\xff\xe1" + struct.pack(">H", len(body) + 2) + body


def main() -> None:
    inputs = [b"", SOI, SOI + b"x", SOI + b"\xff", SOI + SOS, SOI + b"\xff\xe1\x00\x02"]
    for order in ("<", ">"):
        for value in range(11):
            for typ, count in ((3, 1), (2, 1), (3, 2)):
                seed = SOI + app1(order, [entry(order, 0x112, typ, count, value),
                                          entry(order, 0x112, 3, 1, 6)]) + SOS
                inputs += [seed[:k] for k in range(len(seed) + 1)] + [seed]
        many = [entry(order, 0x0001, 3, 1, 1)] * 257 + [entry(order, 0x112, 3, 1, 8)]
        inputs.append(SOI + app1(order, many) + SOS)
        inputs.append(SOI + app1(order, [entry(order, 0x112, 2, 1, 6)])
                      + app1(order, [entry(order, 0x112, 3, 1, 7)]) + SOS)
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(
        json.dumps([list(x) for x in inputs], separators=(",", ":")) + "\n")


if __name__ == "__main__":
    main()
