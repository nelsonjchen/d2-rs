"""Input generator for recipe mathtext-img-png-config (tools/fixtures/mathtext-img.toml).

Writes $D2RS_RECIPE_OUTPUT: a JSON list of byte arrays for the embedded PNG
admission path of d2renderers/d2svgimport (inspectStaticEmbeddedRaster, then
internal/rasterimage.Config and embeddedRasterDecodedBytes). Inputs are 2x3
PNG chunk skeletons with valid CRCs and no pixel data: every bit depth x color
type with PLTE/tRNS/IDAT layouts; extreme dimensions; each IHDR payload byte
set to 0, 1, 2 and 255; PLTE, tRNS and unknown chunks of boundary lengths;
every prefix of a minimal RGBA image and that image with each byte's low bit
flipped (CRCs left stale); and missing, repeated, oversized and APNG chunks.
"""
from __future__ import annotations

import json
import struct
import sys
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

SIG = b"\x89PNG\r\n\x1a\n"


def chunk(kind: bytes, payload: bytes = b"") -> bytes:
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))


def ihdr_payload(width=2, height=3, depth=8, color=6, compression=0, filtering=0, interlace=0) -> bytes:
    return struct.pack(">IIBBBBB", width, height, depth, color, compression, filtering, interlace)


IEND, IDAT = chunk(b"IEND"), chunk(b"IDAT")
PLTE, TRNS = chunk(b"PLTE", b"\x01\x02\x03"), chunk(b"tRNS", b"\xff")


def main() -> None:
    inputs: list[bytes] = []
    for depth in (0, 1, 2, 4, 8, 16, 32):
        for color in range(8):
            header = SIG + chunk(b"IHDR", ihdr_payload(depth=depth, color=color))
            for tail in ([IEND], [IDAT, IEND], [PLTE, IDAT, IEND], [TRNS, IDAT, IEND], [PLTE, TRNS, IEND]):
                inputs.append(header + b"".join(tail))
    for width, height in ((0, 3), (2, 0), (1 << 31, 3), ((1 << 31) - 1, (1 << 31) - 1), ((1 << 32) - 1, 1), (1, 1)):
        inputs.append(SIG + chunk(b"IHDR", ihdr_payload(width, height, depth=16)) + IEND)
    base = ihdr_payload()
    for offset in range(len(base)):
        for value in (0, 1, 2, 255):
            payload = base[:offset] + bytes([value]) + base[offset + 1:]
            inputs.append(SIG + chunk(b"IHDR", payload) + IEND)
    paletted = SIG + chunk(b"IHDR", ihdr_payload(color=3))
    for kind in (b"PLTE", b"tRNS", b"JUNK"):
        for length in (0, 1, 2, 3, 4, 255, 256, 257, 768, 769):
            inputs.append(paletted + chunk(kind, b"\xff" * length) + IDAT + IEND)
    header = SIG + chunk(b"IHDR", base)
    minimal = header + IDAT + IEND
    inputs += [minimal[:k] for k in range(len(minimal) + 1)]
    inputs += [minimal[:k] + bytes([minimal[k] ^ 1]) + minimal[k + 1:] for k in range(len(minimal))]
    inputs += [
        b"\x00" * 8 + IEND,
        SIG + IDAT + IEND,
        header + chunk(b"IHDR", base) + IEND,
        header + chunk(b"JUNK", b"x" * 32769) + IEND,
        header + chunk(b"acTL", b"\x00" * 8) + IEND,
    ]
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(
        json.dumps([list(x) for x in inputs], separators=(",", ":")) + "\n")


if __name__ == "__main__":
    main()
