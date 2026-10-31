#!/usr/bin/env python3
"""Print the ARM64 float instructions Go emits for chosen source lines.

usage: fma-asm.py <asm dump> <file.go> <line>[,<line>...] [function substring]

The dump comes from `go build -gcflags='<pkg>=-S'`. Go's FMADDD Rm, Ra, Rn, Rd
computes Rd = Ra + Rn*Rm (FMSUBD: Ra - Rn*Rm, FNMSUBD: Rn*Rm - Ra), so a site
with two products shows which one is rounded (FMULD) and which is fused.
"""
import re
import sys

dump, source, lines = sys.argv[1], sys.argv[2], {int(x) for x in sys.argv[3].split(",")}
want = sys.argv[4] if len(sys.argv) > 4 else ""
function = None
for text in open(dump):
    m = re.match(r"^(\S+) STEXT", text)
    if m:
        function = m.group(1).split("d2isometricimg.")[-1]
        continue
    m = re.search(r"/" + re.escape(source) + r":(\d+)\)\s+(.*)", text)
    if not m or int(m.group(1)) not in lines or want not in (function or ""):
        continue
    op = m.group(2).split()
    if op and op[0].startswith("F") and op[0] not in ("FUNCDATA",):
        print(f"{function:40.40} {m.group(1):>4} {' '.join(op)}")
