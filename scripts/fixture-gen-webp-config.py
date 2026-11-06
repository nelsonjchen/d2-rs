"""Input generator for recipe mathtext-img-webp-config (tools/fixtures/mathtext-img.toml).

Writes $D2RS_RECIPE_OUTPUT: a JSON list of byte arrays for the embedded WebP
admission path of d2renderers/d2svgimport (inspectStaticEmbeddedRaster, then
internal/rasterimage.Config and embeddedRasterDecodedBytes). Inputs are 2x3
(lossless) and 2x2 (lossy) RIFF skeletons without image data: every prefix of
the VP8/VP8L headers, each header byte replaced by a few values, the
bitstreams behind VP8X, ALPH, unknown and repeated VP8X chunks, VP8X payload
prefixes and byte replacements, every prefix and low-bit flip of the lossless
file, and empty, animated, doubled and oversized files.
"""
from __future__ import annotations

import json
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

VP8 = bytes.fromhex("1000009d012a02000300")  # key frame tag, start code, 2x2
VP8L = bytes.fromhex("2f01800000")  # signature, 2x3
VP8X = bytes.fromhex("00000000" "010000" "020000")  # no flags, canvas 2x3


def chunk(kind: bytes, payload: bytes) -> bytes:
    return kind + struct.pack("<I", len(payload)) + payload + b"\x00" * (len(payload) & 1)


def riff(*chunks: bytes) -> bytes:
    body = b"WEBP" + b"".join(chunks)
    return b"RIFF" + struct.pack("<I", len(body)) + body


def replaced(data: bytes, values: tuple[int, ...]) -> list[bytes]:
    return [data[:k] + bytes([v]) + data[k + 1:] for k in range(len(data)) for v in values]


def main() -> None:
    inputs: list[bytes] = []
    for kind, payload in ((b"VP8 ", VP8), (b"VP8L", VP8L)):
        inputs += [riff(chunk(kind, payload[:k])) for k in range(len(payload) + 1)]
        inputs += [riff(chunk(kind, p)) for p in replaced(payload, (0x00, 0x01, 0x2F, 0xFF))]
        bitstream = chunk(kind, payload)
        inputs.append(riff(bitstream))
        extended = chunk(b"VP8X", VP8X)
        inputs += [riff(extended, bitstream), riff(chunk(b"ALPH", b""), bitstream),
                   riff(chunk(b"JUNK", b"abc"), bitstream), riff(extended, extended, bitstream)]
    lossy, lossless = chunk(b"VP8 ", VP8), chunk(b"VP8L", VP8L)
    inputs += [riff(chunk(b"VP8X", VP8X[:k]), lossy) for k in range(len(VP8X) + 1)]
    inputs.append(riff(chunk(b"VP8X", VP8X), lossy))
    inputs += [riff(chunk(b"VP8X", p), lossless) for p in replaced(VP8X, (0x00, 0x01, 0x02, 0x10, 0xFF))]
    simple = riff(lossless)
    inputs += [simple[:k] for k in range(len(simple) + 1)]
    inputs += [simple[:k] + bytes([simple[k] ^ 1]) + simple[k + 1:] for k in range(len(simple))]
    inputs += [riff(), riff(chunk(b"ANIM", b"")), riff(lossy, lossless),
               riff(chunk(b"JUNK", b"x" * 32769), lossless)]
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(
        json.dumps([list(x) for x in inputs], separators=(",", ":")) + "\n")


if __name__ == "__main__":
    main()
