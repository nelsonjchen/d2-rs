#!/usr/bin/env python3
"""Greedy line-level reduction of a D2 input while a predicate still holds.

The predicate is a shell command run with the candidate file path appended;
exit status 0 means "still interesting". Prints the reduced source.
Example:
  scripts/reduce-d2-witness.py in.d2 -- sh -c 'd2-rs --layout=tala "$0" /dev/null 2>&1 | grep -q "index out of range"'
"""

from __future__ import annotations

import argparse
from pathlib import Path
import subprocess
import tempfile


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("predicate", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    predicate = args.predicate[1:] if args.predicate[:1] == ["--"] else args.predicate
    lines = args.source.read_text().splitlines()
    with tempfile.TemporaryDirectory(prefix="d2-reduce-") as scratch:
        candidate = Path(scratch) / "candidate.d2"

        def interesting(trial: list[str]) -> bool:
            candidate.write_text("\n".join(trial) + "\n")
            return subprocess.run([*predicate, str(candidate)], capture_output=True, timeout=120).returncode == 0

        if not interesting(lines):
            parser.error("predicate does not hold for the original input")
        chunk = max(1, len(lines) // 2)
        while chunk >= 1:
            index, changed = 0, False
            while index < len(lines):
                trial = lines[:index] + lines[index + chunk:]
                if trial and interesting(trial):
                    lines, changed = trial, True
                else:
                    index += chunk
            if not changed:
                chunk //= 2
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
