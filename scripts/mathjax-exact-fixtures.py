#!/usr/bin/env python3
"""Build the d2-rs-math exact-MathJax regression fixture.

Each input is a JSON array of TeX strings. Every expression is rendered by the
pinned Go oracle (tools/mathjax-oracle: github.com/d2lang/mathjax-go v0.1.0
mathjax.Render, which D2 v0.9.0's d2latex.Render calls) and by
d2_rs_math::render_exact (crates/d2-rs-math/examples/math-oracle). The fixture
records, per expression, the SHA-256 of the Go SVG and the Rust outcome class:

* "match":    render_exact returns exactly the Go bytes;
* "refused":  render_exact returns an explicit error (never approximate output);
* "mismatch": render_exact returns different bytes (a known, recorded defect).

The Rust test (src/mathjax_exact_tests.rs) asserts each class, so a fix must
promote its entry and a regression fails. Go-side errors are not recorded.

Usage:
  cargo build --profile diagnostic -p d2-rs-math --example math-oracle
  scripts/mathjax-exact-fixtures.py --go .reference/bin/mathjax-oracle \
      --rust target/diagnostic/examples/math-oracle \
      --output crates/d2-rs-math/tests/fixtures/mathjax-exact-v090.json INPUT.json...
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--go", type=Path, required=True)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("inputs", type=Path, nargs="+")
    args = parser.parse_args()
    expressions: list[str] = []
    for path in args.inputs:
        data = json.loads(path.read_text())
        # A previous fixture may be passed back in to regenerate it.
        if isinstance(data, dict):
            data = [case["tex"] for case in data["cases"]]
        for tex in data:
            if tex not in expressions:
                expressions.append(tex)
    go = json.loads(
        subprocess.run([str(args.go)], input=json.dumps(expressions), capture_output=True, text=True, check=True).stdout
    )
    rust = json.loads(
        subprocess.run(
            [str(args.rust)],
            input=json.dumps([{"expression": tex} for tex in expressions]),
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    )
    cases = []
    for tex, g, r in zip(expressions, go, rust, strict=True):
        if g.get("error"):
            continue
        digest = hashlib.sha256(g["svg"].encode()).hexdigest()
        if r["exact_svg"] is None:
            outcome = "refused"
        elif r["exact_svg"] == g["svg"]:
            outcome = "match"
        else:
            outcome = "mismatch"
        cases.append({"tex": tex, "sha256": digest, "outcome": outcome})
    counts = {k: sum(c["outcome"] == k for c in cases) for k in ("match", "refused", "mismatch")}
    fixture = {
        "oracle": "github.com/d2lang/mathjax-go v0.1.0 mathjax.Render (tools/mathjax-oracle)",
        "counts": counts,
        "cases": cases,
    }
    args.output.write_text(json.dumps(fixture, ensure_ascii=False, indent=0) + "\n")
    print(counts)


if __name__ == "__main__":
    main()
