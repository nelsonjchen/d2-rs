#!/usr/bin/env python3
"""Greedy line/token minimizer for fuzz witnesses (d2-rs vs Go divergence).

usage: fuzz-minimize.py <d2-rs> <go-d2> <findings.jsonl> <witness-dir> <layout>
       [--format svg] [--render-flag FLAG] [--limit 60]

For output, exit and error-text findings, repeatedly drops line chunks
(halving) and then single characters inside short files while the same
divergence kind persists. Error reduction preserves both diagnostic texts,
allowing source coordinates to move.
Prints each minimized input, de-duplicated, with its kind.
"""
import argparse, json, os, re, tempfile
from pathlib import Path
from fuzz_common import first_error, run

def outcome(binary, text, layout, output_format, render_flags):
    with tempfile.TemporaryDirectory(prefix="mn-") as directory:
        Path(directory, "input.d2").write_text(text)
        code, stderr, digest = run(
            binary, directory, layout, "render", output_format, render_flags
        )
        return code, digest, first_error(stderr, directory)

def kind(rust, go, text, layout, output_format="svg", render_flags=()):
    g = outcome(go, text, layout, output_format, render_flags)
    r = outcome(rust, text, layout, output_format, render_flags)
    if "timeout" in (g[0], r[0]): return None
    if (g[0] == 0) != (r[0] == 0): return "exit-diff"
    if g[0] == 0 and g[1] != r[1]: return "output-diff"
    # Preserve the original pair of diagnostics, so reduction cannot switch
    # to an unrelated parser error just because it also differs.
    if g[0] != 0 and g[2] != r[2]:
        # Source coordinates necessarily move as lines/characters disappear.
        diagnostic = lambda message: re.sub(r"input\.d2:\d+:\d+:", "input.d2:<location>:", message)
        return ("error-diff", diagnostic(g[2]), diagnostic(r[2]))
    return None

def minimize(rust, go, text, layout, want, output_format="svg", render_flags=()):
    lines = text.split("\n"); chunk = max(1, len(lines) // 2)
    while chunk >= 1:
        i, changed = 0, False
        while i < len(lines):
            trial = lines[:i] + lines[i + chunk:]
            if trial and kind(
                rust, go, "\n".join(trial), layout, output_format, render_flags
            ) == want:
                lines, changed = trial, True
            else:
                i += chunk
        if not changed: chunk //= 2
    text = "\n".join(lines)
    if len(text) < 400:
        i = 0
        while i < len(text):
            trial = text[:i] + text[i + 1:]
            if trial.strip() and kind(
                rust, go, trial, layout, output_format, render_flags
            ) == want: text = trial
            else: i += 1
    return text

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("rust")
    parser.add_argument("go")
    parser.add_argument("index")
    parser.add_argument("witness_dir")
    parser.add_argument("layout")
    parser.add_argument("--format", default="svg", choices=("svg", "png", "pdf", "pptx", "gif", "txt"))
    parser.add_argument("--render-flag", action="append", default=[])
    parser.add_argument("--limit", type=int, default=60)
    args = parser.parse_args()
    rust, go = args.rust, args.go
    index, wdir, layout = args.index, args.witness_dir, args.layout
    seen = {}
    rows = [json.loads(l) for l in open(index) if json.loads(l)["kind"] in ("output-diff", "exit-diff", "error-diff")][:args.limit]
    for r in rows:
        text = open(os.path.join(wdir, r["file"])).read()
        want = kind(rust, go, text, layout, args.format, args.render_flag)
        if (want[0] if isinstance(want, tuple) else want) != r["kind"]: continue
        small = minimize(
            rust, go, text, layout, want, args.format, args.render_flag
        )
        key = (r["kind"], small.strip())
        seen.setdefault(key, []).append(r["file"])
        print(f"--- {r['kind']} from {r['file']}\n{small}\n", flush=True)
    print("UNIQUE", len(seen))
    for (k, s), files in sorted(seen.items(), key=lambda x: -len(x[1])):
        print(f"=== {k} x{len(files)}\n{s}\n")

if __name__ == "__main__":
    main()
