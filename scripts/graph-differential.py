#!/usr/bin/env python3
"""Seeded differential sweep of generated D2 graphs: d2-rs vs the pinned Go D2.

usage: graph-differential.py <d2-rs> [--go GO] [--family wide|shapes]
       [--cases 400] [--seed 1] [--layouts dagre,elk,tala] [--jobs 8]
       [--minimize] [--out DIR] [--go-mapseed PATH] [--mapseed-seeds 1-16]
       [--record-mapseed]

`shapes` is shape-trace-differential.py's family: non-rectangular shapes at
sizes around the wedge thresholds with parallel, reversed and labeled edges.
`wide` mixes the features ordinary diagrams combine and the upstream corpus
varies little: nested containers and grids, per-container directions, label
and icon positions, near constants, arrowheads and edge label positions,
style keywords, Markdown and code labels, SQL tables and classes, sequence
diagrams, and the --sketch and --theme render flags. Every case renders with
both binaries for each layout; complete output inventories are compared byte
for byte. With --minimize, each mismatch is also reduced line by line to a
smaller input with the same kind of divergence. Before an output difference
counts, the seedable-map oracle (scripts/build-mapseed-oracle.py; --go-mapseed,
default the --go path plus -mapseed) renders it under map seeds 1..16
(--mapseed-seeds) and scripts/mapseed.py classifies it: mapseed-match (d2-rs
equals a seeded Go output; the seeds are recorded), known-mapseed (the same,
for an input in the registry), go-nondeterministic-no-rust-match (Go varies
but d2-rs equals none of its outputs) or mismatch. Without the seeded oracle a
differing case is mapseed-oracle-missing, never a pass. When Go exits nonzero,
d2-rs must exit with the same code, print the same complete stderr (run
directory masked) and leave the same files (none): error-match passes,
error-mismatch fails, and a differing Go error is resampled like an output.
Only a Go signal, panic, timeout or invalid output is unscored (go-crash,
listed as UNSCORED). findings.jsonl has one
record per case that did not plainly match; the summary prints counts per
classification and every failing input. --record-mapseed adds the inputs whose
Go output varied to the registry. The exit status is 1 when any case fails.
"""
import argparse
import concurrent.futures
import hashlib
import importlib.util
import json
import random
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import references  # noqa: E402
import mapseed  # noqa: E402
from fuzz_common import run  # noqa: E402

_spec = importlib.util.spec_from_file_location(
    "shape_trace_differential", Path(__file__).with_name("shape-trace-differential.py"))
shape_trace = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(shape_trace)

SHAPES = ("rectangle", "square", "circle", "oval", "cylinder", "queue", "page", "parallelogram",
          "document", "hexagon", "package", "step", "callout", "stored_data", "person",
          "diamond", "cloud", "c4-person")
DIRECTIONS = ("right", "down", "left", "up")
LABEL_POSITIONS = ("top-left", "top-center", "top-right", "center-left", "center-center",
                   "center-right", "bottom-left", "bottom-center", "bottom-right",
                   "outside-top-left", "outside-top-center", "outside-top-right",
                   "outside-left-top", "outside-left-center", "outside-left-bottom",
                   "outside-right-top", "outside-right-center", "outside-right-bottom",
                   "outside-bottom-left", "outside-bottom-center", "outside-bottom-right")
NEAR = ("top-left", "top-center", "top-right", "center-left", "center-right",
        "bottom-left", "bottom-center", "bottom-right")
ARROWHEADS = ("triangle", "arrow", "diamond", "circle", "box", "cf-one", "cf-many",
              "cf-one-required", "cf-many-required", "cross")
ARROWS = ("->", "->", "->", "<-", "<->", "--")
WORDS = ("api", "web", "db", "cache", "queue", "auth", "worker", "user", "admin", "store",
         "log", "cdn", "search", "billing", "mail")
THEMES = ("0", "1", "3", "4", "5", "6", "8", "100", "101", "102", "103", "104", "105", "200", "300")


def _style(rng):
    options = [
        lambda: f"style.stroke-width: {rng.randint(1, 6)}",
        lambda: f"style.stroke-dash: {rng.randint(0, 6)}",
        lambda: f"style.border-radius: {rng.choice((0, 4, 12, 30))}",
        lambda: f"style.font-size: {rng.choice((8, 12, 16, 22, 30))}",
        lambda: f"style.opacity: {rng.choice((0.3, 0.6, 1))}",
        lambda: "style.shadow: true",
        lambda: "style.3d: true",
        lambda: "style.multiple: true",
        lambda: "style.double-border: true",
        lambda: "style.bold: false",
        lambda: "style.italic: true",
        lambda: "style.underline: true",
        lambda: f'style.fill: "{rng.choice(("#f0e6ff", "#ffe", "red", "transparent"))}"',
    ]
    return rng.choice(options)()


def _label(rng):
    kind = rng.random()
    if kind < 0.08:
        return "|md\n  # Title\n  - one\n  - **two**\n|"
    if kind < 0.12:
        return "|go\n  func main() { fmt.Println(1) }\n|"
    words = " ".join(rng.choice(WORDS) for _ in range(rng.randint(1, 4)))
    return f'"{words}"'


def _node(rng, name, depth, lines, indent, budget):
    """Appends one object (possibly a container) and returns the leaf paths."""
    pad = "  " * indent
    roll = rng.random()
    if depth < 2 and roll < 0.22 and budget > 2:
        lines.append(f"{pad}{name}: {{")
        if rng.random() < 0.3:
            lines.append(f"{pad}  direction: {rng.choice(DIRECTIONS)}")
        if rng.random() < 0.2:
            lines.append(f"{pad}  grid-columns: {rng.randint(1, 3)}")
        if rng.random() < 0.3:
            lines.append(f"{pad}  label.near: {rng.choice(LABEL_POSITIONS)}")
        leaves = []
        for i in range(rng.randint(1, 3)):
            leaves += [f"{name}.{leaf}" for leaf in _node(rng, f"c{i}", depth + 1, lines, indent + 1, budget - 1)]
        lines.append(f"{pad}}}")
        return leaves or [name]
    if roll < 0.27:
        lines.append(f"{pad}{name}: {{")
        lines.append(f"{pad}  shape: sql_table")
        for column in rng.sample(("id: int {constraint: primary_key}", "name: string",
                                  "owner: int {constraint: foreign_key}", "created: timestamp"),
                                 rng.randint(1, 4)):
            lines.append(f"{pad}  {column}")
        lines.append(f"{pad}}}")
        return [name]
    if roll < 0.30:
        lines.append(f"{pad}{name}: {{")
        lines.append(f"{pad}  shape: class")
        lines.append(f'{pad}  "+field": int')
        lines.append(f'{pad}  "-method(a int)": bool')
        lines.append(f"{pad}}}")
        return [name]
    attrs = []
    if rng.random() < 0.6:
        attrs.append(f"shape: {rng.choice(SHAPES)}")
    if rng.random() < 0.35:
        attrs.append(f"label: {_label(rng)}" if rng.random() < 0.1 else
                     f'label: "{" ".join(rng.choice(WORDS) for _ in range(rng.randint(1, 3)))}"')
    if rng.random() < 0.25:
        attrs.append(f"label.near: {rng.choice(LABEL_POSITIONS)}")
    if rng.random() < 0.3:
        attrs.append(_style(rng))
    if rng.random() < 0.08:
        attrs.append(f"tooltip: {rng.choice(WORDS)}")
    if rng.random() < 0.06:
        attrs.append("link: https://example.com")
    if rng.random() < 0.1:
        attrs.append(f"width: {rng.choice((40, 80, 160, 300))}")
    if any(a.startswith("label: |") for a in attrs):
        # Block strings need their own lines.
        lines.append(f"{pad}{name}: {{")
        for attr in attrs:
            lines.append(f"{pad}  {attr}")
        lines.append(f"{pad}}}")
    elif attrs:
        lines.append(f"{pad}{name}: {{ {'; '.join(attrs)} }}")
    else:
        lines.append(f"{pad}{name}")
    return [name]


def generate_wide(rng):
    """One ordinary-looking D2 diagram combining several features."""
    lines = []
    if rng.random() < 0.5:
        lines.append(f"direction: {rng.choice(DIRECTIONS)}")
    if rng.random() < 0.08:
        # A sequence diagram on its own.
        actors = rng.sample(WORDS, rng.randint(2, 4))
        lines.append("shape: sequence_diagram")
        for _ in range(rng.randint(2, 6)):
            a, b = rng.sample(actors, 2)
            label = f": {rng.choice(WORDS)}" if rng.random() < 0.6 else ""
            lines.append(f"{a} -> {b}{label}")
        if rng.random() < 0.4:
            lines.append(f"{actors[0]}.note: {rng.choice(WORDS)}")
        return "\n".join(lines) + "\n"
    if rng.random() < 0.1:
        lines.append(f"grid-rows: {rng.randint(1, 3)}")
    leaves = []
    for i in range(rng.randint(2, 6)):
        leaves += _node(rng, f"n{i}", 0, lines, 0, 4)
    if rng.random() < 0.15:
        lines.append(f"legend: {rng.choice(WORDS)} {{ near: {rng.choice(NEAR)} }}")
    for _ in range(rng.randint(1, len(leaves) + 2)):
        if len(leaves) < 2:
            break
        source, target = rng.sample(leaves, 2)
        label = f": {rng.choice(WORDS)}" if rng.random() < 0.4 else ""
        edge = f"{source} {rng.choice(ARROWS)} {target}{label}"
        attrs = []
        if rng.random() < 0.15:
            attrs.append(f"target-arrowhead.shape: {rng.choice(ARROWHEADS)}")
        if rng.random() < 0.08:
            attrs.append(f"source-arrowhead.shape: {rng.choice(ARROWHEADS)}")
        if label and rng.random() < 0.15:
            attrs.append(f"label.near: {rng.choice(('top-left', 'top-center', 'bottom-right', 'center-center'))}")
        if rng.random() < 0.1:
            attrs.append(_style(rng))
        lines.append(f"{edge} {{ {'; '.join(attrs)} }}" if attrs else edge)
    return "\n".join(lines) + "\n"


def flags_for(rng):
    flags = []
    if rng.random() < 0.15:
        flags.append("--sketch")
    if rng.random() < 0.2:
        flags.append(f"--theme={rng.choice(THEMES)}")
    return tuple(flags)



def outcome(binary, source, layout, flags, root, tag, extra_env=None):
    directory = root / tag
    directory.mkdir()
    (directory / "input.d2").write_text(source)
    return run(binary, directory, layout, "out", "svg", flags, extra_env)


GO_CRASH = "go-crash"
ERROR_MATCH = "error-match"
ERROR_MISMATCH = "error-mismatch"
PROBE_SOURCE = "a -> b\n"


def written_files(directory):
    """Every file a failed run left in its output directory (the input copy excluded)."""
    output = Path(directory) / "out"
    if not output.is_dir():
        return ()
    return tuple((path.relative_to(output).as_posix(), hashlib.sha256(path.read_bytes()).hexdigest())
                 for path in sorted(output.rglob("*")) if path.is_file() and path != output / "input.d2")


def outcome_key(result, directory):
    """Comparable outcome.

    Success: the complete output inventory digest. Failure: the exit code,
    the complete stderr with the run directory masked, and every file the run
    left behind (Go writes none when it fails, so d2-rs must write none).
    A signal, panic, timeout or invalid output is "unusable": no oracle result.
    """
    code, stderr, digest = result
    if code == 0:
        return ("ok", digest)
    if code in ("timeout", "invalid-output") or isinstance(code, int) and code < 0 or "panic:" in stderr:
        return ("unusable", code)
    return ("error", code, stderr.replace(str(directory), "<dir>"), written_files(directory))


def usable_key(key):
    """A seeded run that timed out, crashed or wrote invalid output is not a Go variant."""
    return key[0] != "unusable"


class Resampler:
    """The seeded oracle, seed list and registry of one harness run."""

    def __init__(self, go, oracle=None, seeds=mapseed.DEFAULT_SEEDS, registry=None):
        self.go = go
        self.oracle = mapseed.Oracle(oracle or mapseed.default_oracle(go), go, self.probe)
        self.seeds = tuple(seeds)
        self.registry = registry

    @staticmethod
    def probe(binary):
        with tempfile.TemporaryDirectory(prefix="graph-probe-") as tmp:
            return outcome_key(outcome(binary, PROBE_SOURCE, "dagre", (), Path(tmp), "probe"), Path(tmp) / "probe")

    @staticmethod
    def args(layout, flags):
        return [f"--layout={layout}", *flags, "svg"]

    @staticmethod
    def identity(source, layout, flags):
        return mapseed.identity(source, Resampler.args(layout, flags))

    def known(self, source, layout, flags):
        return bool(self.registry and self.registry.known(self.identity(source, layout, flags)))

    def resolve(self, source, layout, flags, root, go_key, rust_key):
        """Classifies one case from its stock Go and d2-rs keys; seeded runs go under root."""
        def seeded(seed):
            tag = f"go-mapseed-{seed}"
            result = outcome(self.oracle.path, source, layout, flags, root, tag, {mapseed.ENV: str(seed)})
            return outcome_key(result, root / tag)
        return mapseed.resolve([go_key], [rust_key], seeded, self.oracle, self.seeds,
                               self.known(source, layout, flags), usable_key)


def diverges(source, layout, flags, rust, go, resampler):
    """(kind, record): kind is None when the case passes or has no oracle result.

    record carries the classification, and for resolved cases the matching
    map seeds and the number of distinct Go variants seen.
    """
    with tempfile.TemporaryDirectory(prefix="graph-diff-") as tmp:
        root = Path(tmp)
        go_result = outcome(go, source, layout, flags, root, "go")
        rust_result = outcome(rust, source, layout, flags, root, "rust")
        go_key, rust_key = outcome_key(go_result, root / "go"), outcome_key(rust_result, root / "rust")
        if go_key[0] == "unusable":
            # Go crashed, timed out or wrote invalid output: no oracle result.
            return None, {"classification": GO_CRASH, "go_exit": go_result[0], "rust_exit": rust_result[0]}
        if go_key[0] == "ok" and rust_result[0] != 0:
            kind = f"rust-exit-{rust_result[0]}"
            return kind, {"classification": kind}
        known = resampler.known(source, layout, flags)
        if go_key == rust_key and not known:
            return None, {"classification": mapseed.MATCH if go_key[0] == "ok" else ERROR_MATCH}
        # Go's error text, like its output, may follow map order: resample both.
        resolution = resampler.resolve(source, layout, flags, root, go_key, rust_key)
    record = resolution.record()
    record.update(go_exit=go_result[0], rust_exit=rust_result[0])
    if go_key[0] == "error":
        if resolution.classification == mapseed.MATCH:
            record["classification"] = ERROR_MATCH
        elif resolution.classification == mapseed.MISMATCH:
            record["classification"] = ERROR_MISMATCH
    if record["classification"] in PASSING:
        return None, record
    kind = {mapseed.MISMATCH: "output-diff"}.get(record["classification"], record["classification"])
    return kind, record


def minimize(source, layout, flags, rust, go, want, resampler):
    """Drops whole lines (keeping braces balanced) while the divergence persists."""
    lines = source.rstrip("\n").split("\n")
    changed = True
    while changed:
        changed = False
        for i in range(len(lines)):
            line = lines[i]
            if line.rstrip().endswith("{") or line.strip() == "}" or line.strip().startswith("|"):
                continue
            trial = lines[:i] + lines[i + 1:]
            if trial and diverges("\n".join(trial) + "\n", layout, flags, rust, go, resampler)[0] == want:
                lines, changed = trial, True
                break
    return "\n".join(lines) + "\n"


def compare(case, rust, go, layouts, do_minimize, resampler):
    index, source, flags = case
    records = []
    for layout in layouts:
        kind, record = diverges(source, layout, flags, rust, go, resampler)
        small = None
        if kind and do_minimize and kind != mapseed.ORACLE_MISSING:
            small = minimize(source, layout, flags, rust, go, kind, resampler)
        records.append({"case": index, "layout": layout, "flags": list(flags), "kind": kind,
                        "minimized": small, **record})
    return index, source, records


PASSING = mapseed.PASSING | {ERROR_MATCH}


def failing(record):
    """go-crash is unscored (listed, not a failure); everything else must pass."""
    return record["classification"] not in PASSING | {GO_CRASH}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("rust")
    parser.add_argument("--go", default=str(references.GO_ORACLE))
    parser.add_argument("--family", choices=("wide", "shapes"), default="wide")
    parser.add_argument("--cases", type=int, default=400)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--layouts", default="dagre,elk,tala")
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument("--minimize", action="store_true")
    parser.add_argument("--out", default=str(references.REPORTS / "graph-differential"))
    mapseed.add_arguments(parser)
    args = parser.parse_args(argv)
    layouts = [layout for layout in args.layouts.split(",") if layout]
    registry = mapseed.Registry(args.mapseed_registry)
    resampler = Resampler(args.go, args.go_mapseed, args.mapseed_seeds, registry)
    rng = random.Random(args.seed)
    generate = generate_wide if args.family == "wide" else shape_trace.generate
    cases = []
    for i in range(args.cases):
        source = generate(rng)
        cases.append((i, source, flags_for(rng) if args.family == "wide" else ()))
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    records, sources = [], {}
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        work = lambda c: compare(c, args.rust, args.go, layouts, args.minimize, resampler)  # noqa: E731
        for index, source, found in pool.map(work, cases):
            sources[index] = source
            for record in found:
                if record["classification"] not in (mapseed.MATCH, ERROR_MATCH):
                    path = out / f"case-{index:05d}-{record['layout']}.d2"
                    path.write_text(source)
                    if record["minimized"]:
                        path.with_suffix(".min.d2").write_text(record["minimized"])
                    record["input"] = str(path)
                records.append(record)
    findings = sorted((r for r in records if "input" in r), key=lambda r: (r["case"], r["layout"]))
    with open(out / "findings.jsonl", "w") as index_file:
        for finding in findings:
            index_file.write(json.dumps(finding) + "\n")
    for layout in layouts:
        print(mapseed.summary(layout, args.cases, [r for r in records if r["layout"] == layout]))
    for finding in findings:
        label = "FAIL" if failing(finding) else "UNSCORED" if finding["classification"] == GO_CRASH else None
        if label:
            print(f"{label} {finding['classification']}: {finding['input']} ({finding['layout']}"
                  f"{' ' + ' '.join(finding['flags']) if finding['flags'] else ''})")
    if any(r["classification"] == mapseed.ORACLE_MISSING for r in records):
        print(f"error: {resampler.oracle.message()}", file=sys.stderr)
    if args.record_mapseed:
        added = mapseed.record_cases(registry, (
            (resampler.identity(sources[r["case"]], r["layout"], r["flags"]), sources[r["case"]],
             Resampler.args(r["layout"], r["flags"]),
             f"graph-differential.py --family {args.family} --seed {args.seed} case {r['case']} --layouts {r['layout']}",
             mapseed.Resolution(r["classification"], r.get("mapseed_seeds", []), r.get("go_variants", 1)))
            for r in findings), "graph-differential")
        print(f"recorded {added} new map-order-dependent inputs in {registry.path}")
    return 1 if any(failing(r) for r in records) else 0


if __name__ == "__main__":
    sys.exit(main())
