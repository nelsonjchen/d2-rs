#!/usr/bin/env python3
"""Reject missing rows and lost golden matches in a golden tier scoreboard."""
import json
from pathlib import Path
import sys


def delta(before, after):
    assert before["tier"] == after["tier"], "different tiers"
    old = {row["name"]: row for row in before["rows"]}
    new = {row["name"]: row for row in after["rows"]}
    assert len(old) == len(before["rows"]) and len(new) == len(after["rows"]), "duplicate rows"
    missing = sorted(old.keys() - new.keys())
    regressions = sorted(name for name in old.keys() & new.keys()
                         if old[name]["outcome"] == "golden-match" and new[name]["outcome"] != "golden-match")
    regressions += sorted(name + ":assertions" for name in old.keys() & new.keys()
                          if old[name].get("assertion_outcome") == "assertion-match"
                          and new[name].get("assertion_outcome") != "assertion-match")
    improvements = sorted(name for name in old.keys() & new.keys()
                          if old[name]["outcome"] != "golden-match" and new[name]["outcome"] == "golden-match")
    return {"improvements": improvements, "regressions": regressions, "missing": missing}


if __name__ == "__main__":
    result = delta(*(json.loads(Path(path).read_text()) for path in sys.argv[1:3]))
    print(json.dumps(result, indent=2))
    sys.exit(bool(result["regressions"] or result["missing"]))
