#!/usr/bin/env python3
"""Decode-request grid for url-codec-source-v090.json (recipe `d2js-url-codec`).

Nine D2 scripts are raw-DEFLATE encoded three ways (stored, fixed and dynamic
Huffman, via zlib) and URL-base64 wrapped as d2's encoder does; each stream is
replayed valid, truncated, with trailing junk, CRLF-wrapped, and with single
bit flips. Malformed base64 spellings, every one-byte stream, and the 28
`TestStreams` vectors of the manifest Go toolchain's compress/flate tests
complete the grid. Only inputs are produced here; the expected results come
from the pinned d2js `decode`.

    url_codec_cases.py <output.json> [GOROOT]
"""
from __future__ import annotations

import base64
import json
from pathlib import Path
import re
import subprocess
import sys
import zlib

TEXTS = [
    b"",
    b"a -> b",
    b"hello\n" * 100,
    bytes(range(256)),
    b"\xe2\x82",
    b"\xf0\x90\x80",
    b"\xff\xfeabc",
    "a: café\nb: 東京\n".encode(),
    b"abcde" * 8000,
]
BASE64 = ["", "A", "AA", "AAA", "AAAA", "AAAAA", "A===", "AA=", "AA==", "AB==", "AAA=", "AAB=", "AAAA=", "AA=Z",
          "AA=\r\nZ", "AA==Z", "AA==\r\nZ", "!", "a.b", "é", "\r\n", "\r\nA", "AA\r\n==", "AA\n=\n=\n", "+/==", "-_=="]
MODES = {"stored": (0, zlib.Z_DEFAULT_STRATEGY), "fixed": (9, zlib.Z_FIXED), "dynamic": (9, zlib.Z_DEFAULT_STRATEGY)}
MUTATE_MAX_STREAM = 128  # bit flips only for short streams
MUTATE_OFFSETS = 10
MUTATE_BITS = (0, 3, 7)


def b64(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).decode()


def deflate(text: bytes, level: int, strategy: int) -> bytes:
    compressor = zlib.compressobj(level, zlib.DEFLATED, -15, 8, strategy)
    return compressor.compress(text) + compressor.flush()


def go_streams(goroot: Path) -> list[tuple[str, bytes]]:
    source = (goroot / "src/compress/flate/flate_test.go").read_text()
    body = source[source.index("func TestStreams("):]
    body = body[:body.index("\n\tfor i, tc := range testCases")]
    hexes = r'((?:"[0-9a-f]*"\s*\+?\s*)+)'
    entry = re.compile(r'\{\s*"((?:[^"\\]|\\.)*)",\s*' + hexes + r',\s*(' + hexes[1:-1] + r'|"fail"),?\s*\}')
    out = []
    for desc, hexes, _want in entry.findall(body):
        stream = "".join(re.findall(r'"([0-9a-f]*)"', hexes))
        out.append((desc, bytes.fromhex(stream)))
    return out


def cases(goroot: Path) -> list[dict]:
    rows = [{"name": f"base64-{i}", "operation": "decode", "encoded": v} for i, v in enumerate(BASE64)]
    streams = {}
    for i, text in enumerate(TEXTS):
        for mode, (level, strategy) in MODES.items():
            stream = deflate(text, level, strategy)
            streams[(i, mode)] = stream
            rows.append({"name": f"valid-{i}-{mode}", "operation": "decode", "encoded": b64(stream)})
            for k in sorted({k for k in (0, 1, 2, 3, len(stream) // 2, len(stream) - 1) if 0 <= k < len(stream)}):
                rows.append({"name": f"truncated-{i}-{mode}-{k}", "operation": "decode", "encoded": b64(stream[:k])})
            rows.append({"name": f"trailing-{i}-{mode}", "operation": "decode", "encoded": b64(stream + b"junk")})
            encoded = b64(stream)
            if "=" in encoded:
                rows.append({"name": f"newline-{i}-{mode}", "operation": "decode",
                             "encoded": "\r\n".join(encoded[j:j + 2] for j in range(0, len(encoded), 2))})
    for value in range(256):
        rows.append({"name": f"byte-{value}", "operation": "decode", "encoded": b64(bytes([value]))})
    for (i, mode), stream in streams.items():
        if len(stream) > MUTATE_MAX_STREAM:
            continue
        for offset in range(min(len(stream), MUTATE_OFFSETS)):
            for bit in MUTATE_BITS:
                mutated = bytearray(stream)
                mutated[offset] ^= 1 << bit
                rows.append({"name": f"mutated-{i}-{mode}-{offset}-{bit}", "operation": "decode", "encoded": b64(bytes(mutated))})
    for index, (desc, stream) in enumerate(go_streams(goroot)):
        rows.append({"name": f"go-huffman-{index}", "operation": "decode", "encoded": b64(stream), "description": desc})
    return rows


def main() -> None:
    out = Path(sys.argv[1])
    goroot = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(subprocess.check_output(["go", "env", "GOROOT"], text=True).strip())
    out.write_text(json.dumps(cases(goroot), ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
