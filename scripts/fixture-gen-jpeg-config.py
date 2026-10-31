"""Input generator for recipe mathtext-img-jpeg-config (tools/fixtures/mathtext-img.toml).

Writes $D2RS_RECIPE_OUTPUT: a JSON list of byte arrays for the embedded JPEG
admission path of d2renderers/d2svgimport (inspectStaticEmbeddedRaster, then
internal/rasterimage.Config and embeddedRasterDecodedBytes). The inputs are
minimal 2x3 JPEG marker skeletons (no entropy data): SOF0 with 0..5
components behind optional APP0/APP14 segments and with missing or repeated
frames; every SOF0 payload byte of a 3-component frame replaced by a set of
interesting values; foreign segments before the frame; degenerate
dimensions; Exif orientations 1..8; all prefixes of the base image; and an
oversized APP0 segment.
"""
from __future__ import annotations

import json
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

SOI, EOI = b"\xff\xd8", b"\xff\xd9"
SOS = b"\xff\xda\x00\x02"


def segment(marker: int, payload: bytes) -> bytes:
    return bytes([0xFF, marker]) + struct.pack(">H", len(payload) + 2) + payload


def sof0(components: list[tuple[int, int, int]], height: int = 3, width: int = 2) -> bytes:
    payload = struct.pack(">BHHB", 8, height, width, len(components))
    return segment(0xC0, payload + b"".join(bytes(c) for c in components))


def exif_orientation(value: int) -> bytes:
    tiff = b"II*\x00" + struct.pack("<I", 8) + struct.pack("<H", 1)
    tiff += struct.pack("<HHIH", 0x112, 3, 1, value) + b"\x00\x00"
    return segment(0xE1, b"Exif\x00\x00" + tiff)


PREFIXES = [b"", segment(0xE0, b"JFIF\x00"), segment(0xE0, b"other"), segment(0xEE, b"Adobe" + b"\x00" * 7)]
VALUES = [0x00, 0x01, 0x03, 0x04, 0x08, 0x10, 0x11, 0x22, 0x31, 0x44, 0xFF]
FOREIGN = [0x01, 0xC0, 0xC1, 0xC2, 0xC3, 0xC4, 0xDB, 0xDD, 0xE0, 0xEE, 0xFE, 0xFF]


def main() -> None:
    inputs: list[bytes] = []
    for count in range(6):
        frame = sof0([(k + 1, 0x11, 0) for k in range(count)])
        for prefix in PREFIXES:
            for tail in (frame + SOS + EOI, frame + EOI, frame + frame + SOS + EOI):
                inputs.append(SOI + prefix + tail)
    base_frame = sof0([(1, 0x22, 0), (2, 0x11, 1), (3, 0x11, 1)])
    base = SOI + base_frame + SOS + EOI
    for offset in range(4, len(base_frame)):
        for value in VALUES:
            mutated = base_frame[:offset] + bytes([value]) + base_frame[offset + 1:]
            inputs.append(SOI + mutated + SOS + EOI)
    for marker in FOREIGN:
        inputs.append(SOI + segment(marker, b"abc") + base_frame + SOS + EOI)
    for height, width in ((3, 0), (0, 2), (0xFFFF, 0xFFFF)):
        inputs.append(SOI + sof0([(1, 0x11, 0)], height, width) + SOS + EOI)
    for orientation in range(1, 9):
        inputs.append(SOI + exif_orientation(orientation) + base_frame + SOS + EOI)
    inputs += [base[:k] for k in range(len(base) + 1)]
    inputs.append(SOI + segment(0xE0, b"x" * 32770) + base_frame + SOS + EOI)
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(
        json.dumps([list(x) for x in inputs], separators=(",", ":")) + "\n")


if __name__ == "__main__":
    main()
