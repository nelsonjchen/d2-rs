#!/usr/bin/env python3
"""Generate the auditable MathJax normal-font subset used by d2-rs-math.

The input is the exact generated table from mathjax-go v0.1.0.  The script
does not contact a registry or execute JavaScript; callers must provide the
verified local source checkout explicitly.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from pathlib import Path


ENTRY = re.compile(
    r'\s*(0x[0-9A-Fa-f]+):\s+\{Metrics: Metrics\{Height: ([^,]+), '
    r'Depth: ([^,]+), Width: ([^,}]+)(?:[^}]*)\}, Path: '
    r'"((?:\\.|[^"\\])*)"(?:, Content: "((?:\\.|[^"\\])*)")?\},'
)


def selected(codepoint: int) -> bool:
    return (
        0x20 <= codepoint <= 0x7E
        or 0x1D400 <= codepoint <= 0x1D7FF
        or 0x391 <= codepoint <= 0x3FF
        or 0x2100 <= codepoint <= 0x214F
        or 0x2190 <= codepoint <= 0x21FF
        or 0x2200 <= codepoint <= 0x22FF
        or 0x27F5 <= codepoint <= 0x27FC
        or codepoint in (0x00A0, 0x00B0, 0x00B1, 0x00D7, 0x2026, 0x27E8, 0x27E9)
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--table", default="GLYPHS")
    parser.add_argument("--all-glyphs", action="store_true", help="retain every exact path entry in the supplied source table; Content substitutions are a separate renderer contract")
    parser.add_argument("--source-label", default="data_normal_gen.go")
    args = parser.parse_args()

    rows = []
    for code, height, depth, width, path, _content in ENTRY.findall(
        args.source.read_text()
    ):
        codepoint = int(code, 16)
        if args.all_glyphs or selected(codepoint):
            rows.append(
                (
                    codepoint,
                    float(height),
                    float(depth),
                    float(width),
                    bytes(path, "utf-8").decode("unicode_escape"),
                )
            )
    rows.sort(key=lambda row: row[0])

    with args.output.open("w") as output:
        header = """// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Source-derived from github.com/d2lang/mathjax-go v0.1.0 internal/font
// __SOURCE_LABEL__ (MathJax 3.2.2 TeX font data), under Apache-2.0.
// The source file and attribution are recorded in SOURCE.md and
// THIRD_PARTY_NOTICES.md.

#[derive(Clone, Copy)]
pub(crate) struct Glyph {
    pub codepoint: u32,
    pub height: f64,
    pub depth: f64,
    pub width: f64,
    pub path: &'static str,
}

// Kept as a source-shaped table so the translation remains auditable against
// the pinned generated Go table. It covers Basic Latin, Greek, mathematical
// alphanumeric code points, delimiters, arrows, and operators used by the
// common TeX path. Unsupported font variants remain on the explicit fallback.
pub(crate) static __TABLE__: &[Glyph] = &[
""".replace("__SOURCE_LABEL__", args.source_label).replace("__TABLE__", args.table)
        output.write(header)
        for codepoint, height, depth, width, path in rows:
            output.write(
                "    Glyph {\n"
                f"        codepoint: 0x{codepoint:X},\n"
                f"        height: {height:.6},\n"
                f"        depth: {depth:.6},\n"
                f"        width: {width:.6},\n"
                f"        path: {json.dumps(path)},\n"
                "    },\n"
            )
        output.write("];\n")
        corrections=[]
        for line in args.source.read_text().splitlines():
            entry=re.match(r"\s*(0x[0-9A-Fa-f]+):.*?ItalicCorrection: ([^,}]+)",line)
            if entry and (args.all_glyphs or selected(int(entry[1],16))):
                corrections.append((int(entry[1],16),float(entry[2])))
        output.write("\npub(crate) const ITALIC_CORRECTIONS: &[(u32, f64)] = &[\n")
        for codepoint,correction in sorted(corrections):
            output.write(f"    (0x{codepoint:X}, {correction!r}),\n")
        output.write("];\n")
    subprocess.run(["rustfmt", str(args.output)], check=True)
    print(f"wrote {len(rows)} glyphs to {args.output}")


if __name__ == "__main__":
    main()
