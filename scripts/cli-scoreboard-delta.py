#!/usr/bin/env python3
"""Reject lost CLI assertions/artifact matches and missing inventory rows."""
import argparse
import json
from pathlib import Path


def assertion_matches(report):
    # Original test assertions are useful independently of exact artifact bytes
    # (notably PDF timestamps/compression). Keep the stricter artifact tier too.
    if any(run.get("failure") for run in report.get("runs", {}).values()):
        return set()
    return {row["name"] for row in report["cases"]
            if row.get("go") == row.get("rust") == "pass"
            and row["outcome"] not in {"capture-failed", "unsupported-native-writer"}}


def delta(before, after):
    regressions, improvements = [], []
    old_assertions, new_assertions = assertion_matches(before), assertion_matches(after)
    regressions.extend(f"assertions/{name}: pass -> non-passing"
                       for name in sorted(old_assertions - new_assertions))
    improvements.extend(f"assertions/{name}: non-passing -> pass"
                        for name in sorted(new_assertions - old_assertions))
    for tier, passing in [("cases", "contract-match"), ("goldens", "golden-match")]:
        old = {row["name"]: row["outcome"] for row in before[tier]}
        new = {row["name"]: row["outcome"] for row in after[tier]}
        if len(old) != len(before[tier]) or len(new) != len(after[tier]):
            regressions.append(f"{tier}: duplicate inventory keys")
        for key, value in old.items():
            if key not in new or (value == passing and new[key] != passing):
                regressions.append(f"{tier}/{key}: {value} -> {new.get(key, 'MISSING')}")
        for key, value in new.items():
            if value == passing and old.get(key) != passing:
                improvements.append(f"{tier}/{key}: {old.get(key, 'NEW')} -> {value}")
    return {"improvements": improvements, "regressions": regressions}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    args = parser.parse_args()
    result = delta(json.loads(args.before.read_text()), json.loads(args.after.read_text()))
    print(json.dumps(result, indent=2))
    return int(bool(result["regressions"]))


if __name__ == "__main__":
    raise SystemExit(main())
