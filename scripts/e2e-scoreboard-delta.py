#!/usr/bin/env python3
"""Print boards whose e2e scoreboard outcome changed between two reports."""
import json
import sys

old = {(r["engine"], r["name"]): r["outcome"] for r in json.load(open(sys.argv[1]))["rows"]}
new = {(r["engine"], r["name"]): r["outcome"] for r in json.load(open(sys.argv[2]))["rows"]}
good = {"target-match", "svg-match", "error-match"}
regressions = [(k, old.get(k), v) for k, v in new.items() if old.get(k) in good and v not in good]
fixed = [(k, old.get(k), v) for k, v in new.items() if v in good and old.get(k) not in good]
print(f"fixed {len(fixed)}  regressed {len(regressions)}")
for row in regressions:
    print("REGRESSED", *row)
