#!/usr/bin/env python3
"""Mutation fuzzer: d2-rs vs Go on randomly broken corpus inputs.

usage: fuzz-differential.py <d2-rs> [--go GO] [--cases DIR] [--jobs 2000] [--seed 1] [--layout dagre] [--out DIR]

Each job takes a corpus script, applies 1-4 random structural mutations and
renders it with both binaries (SVG, timeout 20 s). Findings, written as
input files under --out with a JSONL index:
  rust-panic     d2-rs exited 101 or printed "panicked"
  rust-signal    d2-rs died on a signal
  rust-hang      d2-rs timed out where Go finished
  exit-diff      one succeeded, the other failed
  output-diff    both succeeded, complete SVG/board inventories differ
  error-diff     both failed with different first error lines (paths masked)
  go/rust-invalid-output  exit 0 without complete, valid SVG output
"""
import argparse, json, os, random, subprocess, tempfile

import references
from fuzz_common import EMPTY_INVENTORY, first_error, run

TOKENS = ["{", "}", "->", "<-", "<->", "--", ":", ".", "*", "**", "|", "||", "`", "\"", "'", ";", "...@x", "@x",
          "$", "${x}", "vars", "classes", "layers", "scenarios", "steps", "shape", "style", "near", "icon",
          "grid-rows", "grid-columns", "label", "width", "height", "direction", "(a -> b)[0]", "&", "!&",
          "null", "_", "#", "\\", "\n", "  ", "é", "漢", "😀", "‍", "‮", "1e309", "-1", "0"]

def mutate(text, rng):
    lines = text.split("\n")
    for _ in range(rng.randint(1, 4)):
        op = rng.randrange(8)
        if not lines:
            lines = [""]
        i = rng.randrange(len(lines))
        if op == 0 and len(lines) > 1:
            del lines[i]
        elif op == 1:
            lines.insert(i, lines[rng.randrange(len(lines))])
        elif op == 2 and len(lines) > 1:
            j = rng.randrange(len(lines)); lines[i], lines[j] = lines[j], lines[i]
        elif op == 3:
            line = lines[i]; k = rng.randint(0, len(line)); lines[i] = line[:k] + rng.choice(TOKENS) + line[k:]
        elif op == 4 and lines[i]:
            line = lines[i]; k = rng.randrange(len(line)); lines[i] = line[:k] + line[k + 1:]
        elif op == 5:
            cut = rng.randint(0, len("\n".join(lines))); return "\n".join(lines)[:cut]
        elif op == 6:
            line = lines[i]; k = rng.randint(0, len(line)); lines[i] = line[:k] + rng.choice(TOKENS) + rng.choice(TOKENS) + line[k:]
        else:
            lines.insert(i, rng.choice(["a -> b", "x: {", "}", "a.style.fill: red", "...@missing", "a: |md\n# h\n|", "*.shape: circle"]))
    return "\n".join(lines)

def main():
    a = argparse.ArgumentParser(); a.add_argument("rust"); a.add_argument("--go", default=str(references.GO_ORACLE)); a.add_argument("--cases", default=str(references.E2E_CASES))
    a.add_argument("--jobs", type=int, default=2000); a.add_argument("--seed", type=int, default=1)
    a.add_argument("--layout", default="dagre"); a.add_argument("--out", default=str(references.REPORTS / "fuzz"))
    o = a.parse_args(); rng = random.Random(o.seed); os.makedirs(o.out, exist_ok=True)
    scripts = []
    for f in sorted(os.listdir(o.cases)):
        if f.endswith(".json"):
            s = json.load(open(os.path.join(o.cases, f))).get("script") or ""
            if s.strip() and "http" not in s:
                scripts.append((f[:-5], s))
    counts, outcomes, index = {}, {}, open(os.path.join(o.out, "findings.jsonl"), "a")
    for job in range(o.jobs):
        case, src = rng.choice(scripts); text = mutate(src, rng)
        d = tempfile.mkdtemp(prefix="fz-"); open(os.path.join(d, "input.d2"), "w").write(text)
        rg, eg, hg = run(o.go, d, o.layout, "go"); rr, er, hr = run(o.rust, d, o.layout, "rust")
        kind = None
        if rr == 101 or "panicked" in er: kind = "rust-panic"
        elif isinstance(rr, int) and rr < 0: kind = "rust-signal"
        elif rr == "timeout" and rg != "timeout": kind = "rust-hang"
        elif rg == "timeout" or rr == "timeout": kind = None
        elif rg == "invalid-output": kind = "go-invalid-output"
        elif rr == "invalid-output": kind = "rust-invalid-output"
        elif (rg == 0) != (rr == 0): kind = "exit-diff"
        elif rg == 0 and hg != hr: kind = "output-diff"
        elif rg != 0 and first_error(eg, d) != first_error(er, d): kind = "error-diff"
        outcome = kind or ("go-timeout" if rg == "timeout" else
                           "empty-output-match" if hg == hr == EMPTY_INVENTORY else "match")
        outcomes[outcome] = outcomes.get(outcome, 0) + 1
        if kind:
            counts[kind] = counts.get(kind, 0) + 1
            name = f"{kind}-{job}-{case}.d2"; open(os.path.join(o.out, name), "w").write(text)
            index.write(json.dumps({"job": job, "kind": kind, "case": case, "file": name, "go_exit": rg, "rust_exit": rr,
                                    "go_err": first_error(eg, d)[:300], "rust_err": first_error(er, d)[:300] or er[-300:]}) + "\n"); index.flush()
        subprocess.run(["rm", "-rf", d])
        if (job + 1) % 250 == 0:
            print(f"{job + 1}/{o.jobs} findings {counts}", flush=True)
    with open(os.path.join(o.out, "summary.json"), "w") as summary:
        json.dump({"seed": o.seed, "layout": o.layout, "jobs": o.jobs,
                   "findings": counts, "outcomes": outcomes}, summary, indent=2)
    print("DONE", counts, "outcomes", outcomes, flush=True)

if __name__ == "__main__":
    main()
