#!/usr/bin/env python3
"""Compare language tiers; missing keys and lost exact matches are failures."""
import json
import sys

old, new = [json.load(open(path)) for path in sys.argv[1:]]
before = {(r["package"], r["name"]): r for r in old["rows"]}
after = {(r["package"], r["name"]): r for r in new["rows"]}
good = {"golden-match", "text-match", "error-match"}
fixed, regressed = [], []
for key in before.keys() | after.keys():
    for tier in ["outcome", "text_outcome"]:
        left, right = before.get(key, {}).get(tier), after.get(key, {}).get(tier)
        if left in good and right not in good:
            regressed.append((key, tier, left, right))
        if right in good and left not in good:
            fixed.append((key, tier, left, right))
print(f"fixed {len(fixed)}  regressed {len(regressed)}")
for row in fixed:
    print("FIXED", *row)
for row in regressed:
    print("REGRESSED", *row)
missing = before.keys() - after.keys()
if missing:
    print("MISSING", sorted(missing))
raise SystemExit(bool(regressed or missing))
