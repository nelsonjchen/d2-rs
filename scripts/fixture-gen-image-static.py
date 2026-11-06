"""Input generator for recipe mathtext-img-svg-image-static (tools/fixtures/mathtext-img.toml).

Writes $D2RS_RECIPE_OUTPUT: a JSON list of {"format", "data"} byte inputs for
d2svgimport.inspectStaticEmbeddedRaster. Each seed below is a minimal
container skeleton for one structural rule of d2renderers/d2svgimport/image.go
(signature, chunk/marker/block framing, animation, frame counts). The sweep for
a seed S is every prefix of S+b"x" (for seeds over 64 bytes only the first 45,
then len/2, len and S+b"x") followed by S with each of its first 32 bytes set
to 0x00 and to 0xff.
The hook runs every input under no, cancelled and expired contexts.
"""
from __future__ import annotations

import json
import os
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

Z4 = b"\x00" * 4


def png_chunk(kind: bytes, payload: bytes = b"") -> bytes:
    # CRCs are zero: the static inspector frames chunks without checking CRCs.
    return struct.pack(">I", len(payload)) + kind + payload + Z4


PNG_SIG = b"\x89PNG\r\n\x1a\n"
GIF_HEAD = b"GIF89a" + b"\x00" * 7  # header + zero logical screen descriptor
GIF_IMAGE = b",\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00"  # descriptor + LZW min size + empty sub-block
JPEG_SCAN = b"\xff\xd8\xff\xda\x00\x02"


def riff(body: bytes) -> bytes:
    return b"RIFF" + struct.pack("<I", 4 + len(body)) + b"WEBP" + body


SEEDS: list[tuple[str, bytes]] = [
    ("png", PNG_SIG + png_chunk(b"IEND")),
    ("png", Z4 + Z4 + png_chunk(b"IHDR", b"\x00" * 13) + png_chunk(b"IEND")),
    ("png", Z4 + Z4 + png_chunk(b"acTL")),
    ("png", Z4 + Z4 + png_chunk(b"fcTL")),
    ("png", Z4 + Z4 + png_chunk(b"fdAT")),
    ("png", Z4 + Z4 + png_chunk(b"IEND", b"x")),
    ("jpeg", b"\xff\xd8\xff\xd9"),
    ("jpeg", JPEG_SCAN + b"abc\xff\x00x\xff\xd0\xff\xd9"),
    ("jpeg", b"\xff\xd8\xff\xd8"),
    ("jpeg", b"\xff\xd8\xff\xe0\x00\x01"),
    ("jpeg", JPEG_SCAN + b"x" * 32770 + b"\xff\xd9"),
    ("jpeg", b"\xff\xd8\xff\x01\xff\xd9"),
    ("gif", GIF_HEAD + GIF_IMAGE + b";"),
    ("gif", GIF_HEAD + b";"),
    ("gif", GIF_HEAD + GIF_IMAGE + GIF_IMAGE + b";"),
    ("gif", GIF_HEAD + b"!\xf9\x02ab\x00" + GIF_IMAGE + b";"),
    ("gif", GIF_HEAD + b",\x00\x00\x00\x00\x00\x00\x00\x00\x80" + b"\x00" * 6 + b"\x02\x00;"),
    ("gif", b"GIF89a\x00\x00\x00\x00\x80" + b"\x00" * 8 + GIF_IMAGE + b";"),
    ("webp", riff(b"VP8 \x01\x00\x00\x00x\x00")),
    ("webp", riff(b"VP8L" + Z4)),
    ("webp", riff(b"")),
    ("webp", riff(b"ANIM" + Z4)),
    ("webp", riff(b"VP8X\x01\x00\x00\x00\x02\x00VP8 \x01\x00\x00\x00x\x00")),
    ("webp", riff(b"VP8 " + Z4 + b"VP8L" + Z4)),
    ("webp", riff(b"JUNK\x03\x00\x00\x00abc\x00VP8 \x03\x00\x00\x00abc\x00")),
]

LONG_SEED = 64  # seeds longer than this get only their first SHORT_PREFIXES prefixes
SHORT_PREFIXES = 45
MUTATION_LIMIT = 32


def sweep(seed: bytes) -> list[bytes]:
    padded = seed + b"x"
    if len(padded) <= LONG_SEED:
        lengths = list(range(len(padded) + 1))
    else:
        lengths = list(range(SHORT_PREFIXES)) + [len(seed) // 2, len(seed), len(padded)]
    out = [padded[:k] for k in lengths]
    for i in range(min(len(seed), MUTATION_LIMIT)):
        for value in (0x00, 0xFF):
            out.append(seed[:i] + bytes([value]) + seed[i + 1:])
    return out


def main() -> None:
    rows = [{"format": fmt, "data": list(data)} for fmt, seed in SEEDS for data in sweep(seed)]
    rows.append({"format": "other", "data": []})
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(json.dumps(rows, separators=(",", ":")) + "\n")


if __name__ == "__main__":
    main()
