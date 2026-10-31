#!/usr/bin/env python3
"""S1 harness: render upstream board.exp.json with the d2svg port, compare to sketch.exp.svg.

This isolates SVG parity from layout parity. Every e2e golden pair is an
exact (target diagram -> SVG) test. Tiers per board:

* svg-match: normalized bytes equal. Only the XML tag-free version string the
  goldens were generated with and the trailing newline are normalized.
* svg-match-except-fonts: equal after masking embedded font payloads
  (base64 WOFF data URIs). Interim tier until the font port lands; never
  reported as svg-match.
* mismatch / render-error / missing-output.

With --go-oracle (the pinned d2svg rendering the same board.exp.json, built
from ../d2-v0.9.0-trace/cmd/d2rs-svgrender), a golden the pinned Go itself
does not reproduce is reported as stale-golden-*: the Rust output is then
compared with the pinned Go output instead, and never counted as svg-match.

Usage: scripts/d2svg-golden.py [--filter REGEX] [--show-diff N]
"""

from __future__ import annotations

import argparse
import references
import collections
import difflib
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
UPSTREAM = ROOT.parent / "d2"
CASES = references.E2E_CASES
FONT_DATA = re.compile(r"data:application/font-woff;base64,[A-Za-z0-9+/=]*")


def normalize(svg: str) -> str:
    svg = re.sub(r'(data-d2-version|d2Version)="[^"]*"', r'\1=""', svg)
    return svg.rstrip("\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--filter", default="")
    parser.add_argument("--engines", default="dagre,elk,tala")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/diagnostic/examples/d2svg-golden")
    parser.add_argument("--output", type=Path, default=ROOT / ".reference/d2svg-golden/latest.json")
    parser.add_argument("--go-oracle", type=Path, default=None, help="pinned Go d2svg renderer of board.exp.json")
    parser.add_argument("--show-diff", type=int, default=0, help="print the first differing token for N mismatches")
    args = parser.parse_args()
    if not args.binary.is_file():
        parser.error("build first: cargo build --profile diagnostic -p d2-rs-render --example d2svg-golden")
    themes = {}
    for path in CASES.glob("*.json"):
        case = json.loads(path.read_text())
        themes[case["name"]] = case["themeID"]
    boards = []
    for golden in sorted((UPSTREAM / "e2etests/testdata").glob("**/board.exp.json")):
        engine = golden.parent.name
        name = str(golden.parent.parent.relative_to(UPSTREAM / "e2etests/testdata"))
        if engine not in args.engines.split(",") or (args.filter and not re.search(args.filter, name)):
            continue
        svg = golden.parent / "sketch.exp.svg"
        if svg.is_file():
            boards.append((name, engine, golden, svg))
    rows = []
    with tempfile.TemporaryDirectory(prefix="d2svg-golden-") as scratch:
        manifest = []
        for index, (name, engine, golden, _) in enumerate(boards):
            manifest.append({"input": str(golden), "output": f"{scratch}/{index}.svg", "theme_id": themes.get(name)})
        (Path(scratch) / "manifest.json").write_text(json.dumps(manifest))
        process = subprocess.run([str(args.binary), f"{scratch}/manifest.json"], capture_output=True, text=True)
        if process.returncode != 0:
            print(process.stderr[-3000:], file=sys.stderr)
            return 1
        if args.go_oracle:
            go_manifest = [dict(row, output=row["output"] + ".go") for row in manifest]
            (Path(scratch) / "go-manifest.json").write_text(json.dumps(go_manifest))
            process = subprocess.run([str(args.go_oracle), f"{scratch}/go-manifest.json"], capture_output=True, text=True)
            if process.returncode != 0:
                print(process.stderr[-3000:], file=sys.stderr)
                return 1
        shown = 0
        for index, (name, engine, _, svg_path) in enumerate(boards):
            row = {"name": name, "engine": engine}
            out = Path(f"{scratch}/{index}.svg")
            err = Path(f"{scratch}/{index}.svg.err")
            expected = normalize(svg_path.read_text())
            stale = False
            go_out = Path(f"{scratch}/{index}.svg.go")
            if args.go_oracle and go_out.is_file():
                go_svg = normalize(go_out.read_text())
                if go_svg != expected:
                    stale = True
                    expected = go_svg
            go_err = Path(f"{scratch}/{index}.svg.go.err")
            if args.go_oracle and go_err.is_file():
                # The pinned Go cannot render this golden's JSON either.
                row["go_error"] = go_err.read_text()[:300]
                row["outcome"] = "stale-golden-both-error" if err.is_file() else "stale-golden-go-error"
            elif err.is_file():
                row["outcome"] = "render-error"
                row["error"] = err.read_text()[:300]
            elif not out.is_file():
                row["outcome"] = "missing-output"
            else:
                actual = normalize(out.read_text())
                if actual == expected:
                    row["outcome"] = "stale-golden-go-match" if stale else "svg-match"
                elif FONT_DATA.sub("FONT", actual) == FONT_DATA.sub("FONT", expected):
                    row["outcome"] = "svg-match-except-fonts"
                else:
                    row["outcome"] = "stale-golden-go-mismatch" if stale else "mismatch"
                    a = FONT_DATA.sub("FONT", actual)
                    e = FONT_DATA.sub("FONT", expected)
                    at = next((i for i, (x, y) in enumerate(zip(a, e)) if x != y), min(len(a), len(e)))
                    row["first_diff"] = at
                    row["context"] = {"expected": e[max(0, at - 80):at + 120], "actual": a[max(0, at - 80):at + 120]}
                    if shown < args.show_diff:
                        shown += 1
                        print(f"--- {engine} {name} @ {at}\n  exp: {row['context']['expected']!r}\n  got: {row['context']['actual']!r}")
            rows.append(row)
    summary = collections.Counter(row["outcome"] for row in rows)
    errors = collections.Counter(row["error"].split("\n")[0][:80] for row in rows if row["outcome"] == "render-error")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps({"boards": len(rows), "summary": dict(summary), "rows": rows}, indent=1) + "\n")
    print(dict(sorted(summary.items())), "of", len(rows))
    for message, count in errors.most_common(12):
        print(f"  {count:4d} render-error: {message}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
