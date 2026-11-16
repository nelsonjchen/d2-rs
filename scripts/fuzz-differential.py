#!/usr/bin/env python3
"""Mutation fuzzer: d2-rs vs Go on randomly broken corpus inputs.

usage: fuzz-differential.py <d2-rs> [--go GO] [--cases DIR] [--jobs 2000]
       [--seed 1] [--layout dagre] [--format svg]
       [--render-flag=--sketch] [--out DIR]

Each job takes a corpus script, applies 1-4 random structural mutations and
renders it with both binaries. The output format and exact CLI render flags
are selectable, and the corpus includes upstream .d2 files and raw `script:`
literals from Go tests. Findings, written as
input files under --out with a JSONL index:
  rust-panic     d2-rs exited 101 or printed "panicked"
  rust-signal    d2-rs died on a signal
  rust-hang      d2-rs timed out where Go finished
  exit-diff      one succeeded, the other failed
  output-diff    both succeeded, complete SVG/board inventories differ
  error-diff     both failed with different first error lines (paths masked)
  go/rust-invalid-output  exit 0 without complete, valid SVG output

Inputs where pinned Go panics or dies on a signal have no oracle result. They
are retained in skips.jsonl with Rust's outcome, but do not count as findings.

An exit-diff, output-diff or error-diff is first resampled with the
seedable-map oracle (--go-mapseed, default the --go path plus -mapseed; for
PDF a clock+mapseed build from build-cli-clock-oracle.py --mapseed) under
--mapseed-seeds (default 1-16) and classified by scripts/mapseed.py. A
mapseed-match or known-mapseed is an outcome, not a finding; a
go-nondeterministic-no-rust-match or mapseed-oracle-missing is a finding of
that kind. Findings carry the classification, matching seeds and Go variants.
"""
import argparse, json, os, random, re, subprocess, sys, tempfile
from pathlib import Path

import mapseed
import references
from fuzz_common import EMPTY_INVENTORY, first_error, run

TOKENS = ["{", "}", "->", "<-", "<->", "--", ":", ".", "*", "**", "|", "||", "`", "\"", "'", ";", "...@x", "@x",
          "$", "${x}", "vars", "classes", "layers", "scenarios", "steps", "shape", "style", "near", "icon",
          "grid-rows", "grid-columns", "label", "width", "height", "direction", "(a -> b)[0]", "&", "!&",
          "null", "_", "#", "\\", "\n", "  ", "é", "漢", "😀", "‍", "‮", "1e309", "-1", "0"]
SCRIPT = re.compile(r"\bscript\s*:\s*`([^`]*)`", re.S)
FORMATS = ("svg", "png", "pdf", "pptx", "gif", "txt")
MUTATION_FAMILIES = ("structural", "text")
TEXT_BLOCKS = (
    '"__fuzz_text_md": |md\n# Heading *emphasis*\n\n- first item\n- second **bold** item\n\n> quoted `code`\n|',
    '"__fuzz_text_code": |rust\nfn main() { println!("héllo 😀"); }\n|',
    '"__fuzz_text_latex": |latex\n\\\\frac{x_1 + y^2}{z} = \\\\sqrt{n}\n|',
    '"__fuzz_text_unicode": "ગુજરાતી é 👩‍💻 مرحبا"',
    '"__fuzz_text_link": {\n  label: "linked tooltip"\n  link: https://example.com/docs?q=d2\n  tooltip: "line one\\nline two 漢"\n}',
)
GO_PANIC = re.compile(r"(?:^|\n)panic:")


def go_crashed(exit_code, stderr):
    return (isinstance(exit_code, int) and exit_code < 0) or bool(GO_PANIC.search(stderr))


def rust_outcome(exit_code, stderr):
    if exit_code == 101 or "panicked" in stderr:
        return "rust-panic"
    if isinstance(exit_code, int) and exit_code < 0:
        return "rust-signal"
    if exit_code == "timeout":
        return "rust-hang"
    if exit_code == "invalid-output":
        return "rust-invalid-output"
    return f"rust-exit-{exit_code}"

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


def mutate_text(text, rng):
    """Add realistic text constructs without perturbing structural queues."""
    additions = rng.sample(TEXT_BLOCKS, rng.randint(1, min(3, len(TEXT_BLOCKS))))
    separator = "" if not text or text.endswith("\n") else "\n"
    return text + separator + "\n".join(additions) + "\n"

def parse_args(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("rust")
    parser.add_argument("--go", default=str(references.GO_ORACLE))
    parser.add_argument("--cases", default=str(references.E2E_CASES))
    parser.add_argument(
        "--upstream",
        default=str((Path(__file__).resolve().parents[1] / references.MANIFEST["d2"]["checkout"]).resolve()),
    )
    parser.add_argument("--jobs", type=int, default=2000)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--layout", choices=("dagre", "elk", "tala"), default="dagre")
    parser.add_argument("--format", choices=FORMATS, default="svg")
    parser.add_argument("--mutation-family", choices=MUTATION_FAMILIES, default="structural")
    parser.add_argument(
        "--render-flag",
        action="append",
        default=[],
        help="exact render flag, normally passed as --render-flag=--sketch",
    )
    parser.add_argument("--out", default=str(references.REPORTS / "fuzz"))
    mapseed.add_arguments(parser)
    return parser.parse_args(argv)


def outcome_key(exit_code, stderr, digest, directory):
    """What the kinds below compare: the inventory, or the masked first error line."""
    if exit_code == 0:
        return ("ok", digest)
    if exit_code in ("timeout", "invalid-output") or go_crashed(exit_code, stderr):
        return ("unusable", exit_code)
    return ("error", first_error(stderr, directory))


def usable_key(key):
    return key[0] != "unusable"


def mapseed_oracle(o):
    def probe(binary):
        directory = tempfile.mkdtemp(prefix="fz-probe-")
        try:
            Path(directory, "input.d2").write_text("a -> b\n")
            return outcome_key(*run(binary, directory, o.layout, "probe", o.format, o.render_flag), directory)
        finally:
            subprocess.run(["rm", "-rf", directory])
    return mapseed.Oracle(o.go_mapseed or mapseed.default_oracle(o.go), o.go, probe,
                          mapseed.CLOCK_BUILD_COMMAND if o.format == "pdf" else mapseed.BUILD_COMMAND)


def resolve(o, oracle, registry, text, d, go_key, rust_key):
    args = [f"--layout={o.layout}", *o.render_flag, o.format]
    known = registry.known(mapseed.identity(text, args))

    def seeded(seed):
        name = f"go-mapseed-{seed}"
        code, stderr, digest = run(oracle.path, d, o.layout, name, o.format, o.render_flag, {mapseed.ENV: str(seed)})
        return outcome_key(code, stderr, digest, d)
    return mapseed.resolve([go_key], [rust_key], seeded, oracle, o.mapseed_seeds, known, usable_key)


def load_scripts(cases, upstream):
    scripts = []
    for f in sorted(os.listdir(cases)):
        if f.endswith(".json"):
            s = json.loads(Path(cases, f).read_text()).get("script") or ""
            if s.strip() and "http" not in s:
                scripts.append((f"e2e:{f[:-5]}", s))
    upstream = Path(upstream)
    for path in sorted(upstream.rglob("*.d2")):
        source = path.read_text(errors="replace")
        if source.strip() and "http" not in source:
            scripts.append((f"upstream:{path.relative_to(upstream)}", source))
    for path in sorted(upstream.rglob("*_test.go")):
        source = path.read_text(errors="replace")
        for index, match in enumerate(SCRIPT.finditer(source)):
            script = match.group(1)
            if script.strip() and "http" not in script:
                scripts.append((f"upstream:{path.relative_to(upstream)}:script:{index}", script))
    return scripts


def main(argv=None):
    o = parse_args(argv)
    if o.format == "pdf" and Path(o.go).resolve() == Path(references.GO_ORACLE).resolve():
        raise SystemExit(
            "PDF fuzzing requires --go with the clock-injected pinned oracle "
            "from scripts/build-cli-clock-oracle.py"
        )
    rng = random.Random(o.seed)
    os.makedirs(o.out, exist_ok=True)
    scripts = load_scripts(o.cases, o.upstream)
    if not scripts:
        raise SystemExit("no fuzz corpus inputs found")
    counts, outcomes = {}, {}
    go_crash_rust_outcomes = {}
    oracle, registry, recorded = mapseed_oracle(o), mapseed.Registry(o.mapseed_registry), []
    index = open(os.path.join(o.out, "findings.jsonl"), "a")
    skips = open(os.path.join(o.out, "skips.jsonl"), "a")
    for job in range(o.jobs):
        case, src = rng.choice(scripts)
        text = mutate_text(src, rng) if o.mutation_family == "text" else mutate(src, rng)
        d = tempfile.mkdtemp(prefix="fz-"); open(os.path.join(d, "input.d2"), "w").write(text)
        rg, eg, hg = run(o.go, d, o.layout, "go", o.format, o.render_flag)
        rr, er, hr = run(o.rust, d, o.layout, "rust", o.format, o.render_flag)
        kind = None
        go_crash = go_crashed(rg, eg)
        if go_crash:
            outcome = "go-crash"
            rust_status = rust_outcome(rr, er)
            go_crash_rust_outcomes[rust_status] = go_crash_rust_outcomes.get(rust_status, 0) + 1
        else:
            if rr == 101 or "panicked" in er: kind = "rust-panic"
            elif isinstance(rr, int) and rr < 0: kind = "rust-signal"
            elif rr == "timeout" and rg != "timeout": kind = "rust-hang"
            elif rg == "timeout" or rr == "timeout": kind = None
            elif rg == "invalid-output": kind = "go-invalid-output"
            elif rr == "invalid-output": kind = "rust-invalid-output"
            elif (rg == 0) != (rr == 0): kind = "exit-diff"
            elif rg == 0 and hg != hr: kind = "output-diff"
            elif rg != 0 and first_error(eg, d) != first_error(er, d): kind = "error-diff"
            resolution = None
            if kind in ("exit-diff", "output-diff", "error-diff") or kind is None and rg != "timeout" and \
                    registry.known(mapseed.identity(text, [f"--layout={o.layout}", *o.render_flag, o.format])):
                resolution = resolve(o, oracle, registry, text, d, outcome_key(rg, eg, hg, d), outcome_key(rr, er, hr, d))
                if resolution.passed:
                    kind = None
                elif resolution.classification != mapseed.MISMATCH:
                    kind = resolution.classification
                if resolution.go_variants > 1:
                    recorded.append((mapseed.identity(text, [f"--layout={o.layout}", *o.render_flag, o.format]), text,
                                     [f"--layout={o.layout}", *o.render_flag, o.format],
                                     f"fuzz-differential.py --seed {o.seed} --layout {o.layout} --format {o.format}"
                                     f" job {job} ({case})", resolution))
            outcome = kind or ("go-timeout" if rg == "timeout" else
                               resolution.classification if resolution and resolution.classification != mapseed.MATCH else
                               "empty-output-match" if hg == hr == EMPTY_INVENTORY else "match")
        outcomes[outcome] = outcomes.get(outcome, 0) + 1
        if go_crash:
            safe_case = re.sub(r"[^A-Za-z0-9_.-]+", "_", case)
            name = f"go-crash-{job}-{safe_case}.d2"; open(os.path.join(o.out, name), "w").write(text)
            skips.write(json.dumps({"job": job, "outcome": outcome, "rust_outcome": rust_status,
                                    "case": case, "file": name, "go_exit": rg, "rust_exit": rr,
                                    "mutation_family": o.mutation_family,
                                    "format": o.format, "render_flags": o.render_flag,
                                    "go_err": first_error(eg, d)[:300],
                                    "rust_err": first_error(er, d)[:300] or er[-300:]}) + "\n")
            skips.flush()
        elif kind:
            counts[kind] = counts.get(kind, 0) + 1
            safe_case = re.sub(r"[^A-Za-z0-9_.-]+", "_", case)
            name = f"{kind}-{job}-{safe_case}.d2"; open(os.path.join(o.out, name), "w").write(text)
            index.write(json.dumps({"job": job, "kind": kind, "case": case, "file": name, "go_exit": rg, "rust_exit": rr,
                                    "mutation_family": o.mutation_family,
                                    "format": o.format, "render_flags": o.render_flag,
                                    "go_err": first_error(eg, d)[:300], "rust_err": first_error(er, d)[:300] or er[-300:],
                                    **(resolution.record() if resolution else {})}) + "\n"); index.flush()
            if kind in (mapseed.NO_RUST_MATCH, mapseed.ORACLE_MISSING):
                print(f"FAIL {kind}: {os.path.join(o.out, name)}", flush=True)
        subprocess.run(["rm", "-rf", d])
        if (job + 1) % 250 == 0:
            print(f"{job + 1}/{o.jobs} findings {counts}", flush=True)
    with open(os.path.join(o.out, "summary.json"), "w") as summary:
        json.dump({"seed": o.seed, "layout": o.layout, "format": o.format,
                   "mutation_family": o.mutation_family,
                   "render_flags": o.render_flag, "corpus_inputs": len(scripts), "jobs": o.jobs,
                   "findings": counts, "outcomes": outcomes,
                   "go_crash_rust_outcomes": go_crash_rust_outcomes}, summary, indent=2)
    print("DONE", counts, "outcomes", outcomes, flush=True)
    if counts.get(mapseed.ORACLE_MISSING):
        print(f"error: {oracle.message()}", file=sys.stderr)
    if o.record_mapseed:
        added = mapseed.record_cases(registry, recorded, "fuzz-differential")
        print(f"recorded {added} new map-order-dependent inputs in {registry.path}")

if __name__ == "__main__":
    main()
