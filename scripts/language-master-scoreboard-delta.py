#!/usr/bin/env python3
"""Reject lost/missing master inline compiler contracts; uncovered tests never pass."""
import json
import sys

def delta(before, after):
    assert before["source_commit"] == after["source_commit"], "source pin changed"
    old = {row["name"]:row for row in before["rows"]}
    new = {row["name"]:row for row in after["rows"]}
    assert old.keys() == new.keys(), "case inventory changed"
    regressions = [name for name in old if old[name]["outcome"] == "match" and new[name]["outcome"] != "match"]
    gains = [name for name in old if old[name]["outcome"] != "match" and new[name]["outcome"] == "match"]
    return {"gains":gains, "regressions":regressions}

if __name__ == "__main__":
    result = delta(*(json.load(open(path)) for path in sys.argv[1:3]))
    print(json.dumps(result, indent=2))
    raise SystemExit(bool(result["regressions"]))
