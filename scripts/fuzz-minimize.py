#!/usr/bin/env python3
"""Greedy line/token minimizer for fuzz witnesses (d2-rs vs Go divergence).

usage: fuzz-minimize.py <d2-rs> <go-d2> <findings.jsonl> <witness-dir> <layout>
       [--format svg] [--render-flag FLAG] [--limit 60]

For output, exit and error-text findings, repeatedly drops line chunks
(halving) and then single characters inside short files while the same
divergence kind persists. Error reduction preserves both diagnostic texts,
allowing source coordinates to move.
Prints each minimized input, de-duplicated, with its kind.

go-nondeterministic-no-rust-match findings are reduced with the seedable-map
oracle (--go-mapseed, default the go path plus -mapseed; --mapseed-seeds,
default 1-16): a trial keeps the kind while Go still gives two or more
outputs across the seeds and d2-rs equals none of them. Stock Go runs do not
enter that predicate. mapseed-oracle-missing findings are skipped with a
message (build the seeded oracle and rerun fuzz-differential.py first), as
are no-rust-match findings when the seeded oracle is absent.
"""
import argparse, json, os, re, sys, tempfile
from pathlib import Path
import mapseed
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

def seeded_key(binary, text, layout, output_format, render_flags, seed=None):
    """One run's comparable outcome: inventory digest, or the first error line."""
    with tempfile.TemporaryDirectory(prefix="mn-") as directory:
        Path(directory, "input.d2").write_text(text)
        code, stderr, digest = run(binary, directory, layout, "render", output_format, render_flags,
                                   {mapseed.ENV: str(seed)} if seed is not None else None)
        if code == 0:
            return ("ok", digest)
        if code in ("timeout", "invalid-output") or isinstance(code, int) and code < 0:
            return None
        return ("error", code, first_error(stderr, directory))


def no_rust_match(rust, oracle, text, layout, output_format="svg", render_flags=(), seeds=mapseed.DEFAULT_SEEDS):
    """NO_RUST_MATCH while Go varies across the seeds and d2-rs equals none of its outputs."""
    rust_key = seeded_key(rust, text, layout, output_format, render_flags)
    if rust_key is None:
        return None
    variants = []
    for seed in seeds:
        key = seeded_key(oracle, text, layout, output_format, render_flags, seed)
        if key is not None and key not in variants:
            variants.append(key)
    return mapseed.NO_RUST_MATCH if len(variants) > 1 and rust_key not in variants else None


def minimize(rust, go, text, layout, want, output_format="svg", render_flags=(), predicate=None):
    lines = text.split("\n"); chunk = max(1, len(lines) // 2)
    while chunk >= 1:
        i, changed = 0, False
        while i < len(lines):
            trial = lines[:i] + lines[i + chunk:]
            if trial and (predicate or kind)(
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
            if trial.strip() and (predicate or kind)(
                rust, go, trial, layout, output_format, render_flags
            ) == want: text = trial
            else: i += 1
    return text

def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("rust")
    parser.add_argument("go")
    parser.add_argument("index")
    parser.add_argument("witness_dir")
    parser.add_argument("layout")
    parser.add_argument("--format", default="svg", choices=("svg", "png", "pdf", "pptx", "gif", "txt"))
    parser.add_argument("--render-flag", action="append", default=[])
    parser.add_argument("--limit", type=int, default=60)
    parser.add_argument("--go-mapseed", default=None, help="seedable-map Go oracle (default: the go path plus -mapseed)")
    parser.add_argument("--mapseed-seeds", type=mapseed.parse_seeds, default=mapseed.DEFAULT_SEEDS)
    args = parser.parse_args(argv)
    rust, go = args.rust, args.go
    index, wdir, layout = args.index, args.witness_dir, args.layout
    oracle = Path(args.go_mapseed) if args.go_mapseed else mapseed.default_oracle(go)
    seen = {}
    findings = [json.loads(l) for l in open(index)]
    missing = [r for r in findings if r["kind"] == mapseed.ORACLE_MISSING]
    if missing:
        print(f"skipping {len(missing)} {mapseed.ORACLE_MISSING} findings: build the seeded oracle "
              f"({mapseed.BUILD_COMMAND}) and rerun fuzz-differential.py to classify them", file=sys.stderr)
    wanted = ("output-diff", "exit-diff", "error-diff", mapseed.NO_RUST_MATCH)
    rows = [r for r in findings if r["kind"] in wanted][:args.limit]
    if any(r["kind"] == mapseed.NO_RUST_MATCH for r in rows) and not oracle.is_file():
        print(f"skipping {mapseed.NO_RUST_MATCH} findings: seedable-map Go oracle not found: {oracle}; "
              f"build it with: {mapseed.BUILD_COMMAND}", file=sys.stderr)
        rows = [r for r in rows if r["kind"] != mapseed.NO_RUST_MATCH]
    seeds = tuple(args.mapseed_seeds)

    def seeded_predicate(rust, go, text, layout, output_format, render_flags):
        return no_rust_match(rust, oracle, text, layout, output_format, render_flags, seeds)
    for r in rows:
        text = open(os.path.join(wdir, r["file"])).read()
        predicate = seeded_predicate if r["kind"] == mapseed.NO_RUST_MATCH else None
        want = (predicate or kind)(rust, go, text, layout, args.format, args.render_flag)
        if (want[0] if isinstance(want, tuple) else want) != r["kind"]: continue
        small = minimize(
            rust, go, text, layout, want, args.format, args.render_flag, predicate
        )
        key = (r["kind"], small.strip())
        seen.setdefault(key, []).append(r["file"])
        print(f"--- {r['kind']} from {r['file']}\n{small}\n", flush=True)
    print("UNIQUE", len(seen))
    for (k, s), files in sorted(seen.items(), key=lambda x: -len(x[1])):
        print(f"=== {k} x{len(files)}\n{s}\n")

if __name__ == "__main__":
    main()
