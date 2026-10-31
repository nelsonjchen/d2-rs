#!/usr/bin/env python3
"""Print the ARM64 contraction sites for one Go file with their source lines.

usage: fma-lines.py <go file under ../d2/d2renderers> [table]
Each line: lineno  ops  enclosing-function  | source text.
"""
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
target = sys.argv[1]
tables = [ROOT / ".reference/isometric-port/fma-sites.txt", ROOT / ".reference/isometric-port/fma-sites-foundations.txt"]
source = (ROOT.parent / "d2" / target).read_text().splitlines()
rows = []
for table in tables:
    for line in table.read_text().splitlines():
        if line.startswith("#"):
            continue
        location, function, *ops = line.split()
        path, number = location.rsplit(":", 1)
        if path == target:
            rows.append((int(number), " ".join(ops), function.rsplit(".", 1)[-1] if "." in function else function))
for number, ops, function in sorted(rows):
    print(f"{number:5} {ops:22} {function:34} | {source[number - 1].strip()}")
