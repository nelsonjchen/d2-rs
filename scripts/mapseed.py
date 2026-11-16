"""Map-seed resolution of Go-vs-Rust differences, shared by the differential harnesses.

Go randomizes every map's hash seed and `range` start offset, so a few
layouts (dagre's shiftReachableDown ranges over a map) give more than one Go
output for one input. scripts/build-mapseed-oracle.py builds the pinned CLI
with seedable map order: with D2RS_GO_MAPSEED=<n>, maps draw their randomness
from a counter mixed with n, so a run is reproducible per seed.

When stock Go repeats disagree, or d2-rs differs from stock Go, a harness
calls resolve(). It renders the case with the seeded oracle under every seed
of a bounded list (default 1..16), using the harness's own capture (same
arguments, environment and complete output inventory), and classifies:

  match              stock Go and every d2-rs repeat agree; no seeds run
  mapseed-match      d2-rs equals the seeded output of at least one seed
                     (the record names the seeds and the Go variants seen)
  known-mapseed      mapseed-match on an input listed in the registry
  go-nondeterministic-no-rust-match
                     Go varies, but d2-rs equals none of its seeded
                     outputs: a d2-rs bug
  mismatch           Go agrees with itself under every seed and d2-rs
                     differs, or d2-rs disagrees with itself
  mapseed-oracle-missing
                     the case needs the seeded oracle, which is absent or
                     not built from the same source as --go: a failure

Only match, mapseed-match and known-mapseed pass (plus each harness's own
error-match rules). A d2-rs output equal only to a stock Go repeat, never to
a seeded one, does not resolve a Go-vs-Go disagreement.

The registry (tools/fixtures/inputs/mapseed-cases.json) lists inputs known to
be map-order dependent: an identity (SHA-256 of the source and the CLI
arguments), a locator and a reason. It is written only by a harness run with
--record-mapseed, never automatically, and stores no oracle output. A listed
input still runs every seed: the entry changes the label, never the verdict.
"""
from __future__ import annotations

import datetime
import hashlib
import json
import os
import subprocess
import sys
import threading
from dataclasses import dataclass, field
from pathlib import Path

import references

ENV = "D2RS_GO_MAPSEED"
DEFAULT_SEEDS = tuple(range(1, 17))
REGISTRY = references.ROOT / "tools/fixtures/inputs/mapseed-cases.json"
RUNTIME_PATCH = references.ROOT / "tools/patches/go1.27-runtime-mapseed.patch"

MATCH = "match"
MAPSEED_MATCH = "mapseed-match"
KNOWN_MAPSEED = "known-mapseed"
NO_RUST_MATCH = "go-nondeterministic-no-rust-match"
MISMATCH = "mismatch"
ORACLE_MISSING = "mapseed-oracle-missing"
CLASSIFICATIONS = (MATCH, MAPSEED_MATCH, KNOWN_MAPSEED, NO_RUST_MATCH, MISMATCH, ORACLE_MISSING)
PASSING = frozenset({MATCH, MAPSEED_MATCH, KNOWN_MAPSEED})

BUILD_COMMAND = "uv run --no-project python scripts/build-mapseed-oracle.py"
CLOCK_BUILD_COMMAND = ("uv run --no-project python scripts/build-cli-clock-oracle.py --mapseed "
                       "--output <fresh path>, then pass it as --go-mapseed")


def runtime_overlay(scratch, env):
    """Patched runtime/rand.go under scratch plus a `go build -overlay` file for it.

    The installed toolchain is never edited. Returns the overlay JSON path.
    """
    goroot = Path(subprocess.check_output(["go", "env", "GOROOT"], env=env, text=True).strip())
    tree = Path(scratch)
    rand = tree / "src/runtime/rand.go"
    rand.parent.mkdir(parents=True)
    rand.write_bytes((goroot / "src/runtime/rand.go").read_bytes())
    subprocess.run(["git", "apply", str(RUNTIME_PATCH)], cwd=tree, check=True)
    overlay = tree / "overlay.json"
    overlay.write_text(json.dumps({"Replace": {str(goroot / "src/runtime/rand.go"): str(rand)}}))
    return overlay


def default_oracle(go):
    """The seeded build that pairs with `go`: the same path plus -mapseed."""
    go = Path(go)
    return go.with_name(go.name + "-mapseed")


def parse_seeds(text):
    """"1-16", "3,9" or "1-4,9" as a tuple of distinct positive seeds."""
    seeds = []
    for part in str(text).split(","):
        part = part.strip()
        if not part:
            continue
        low, _, high = part.partition("-")
        first, last = int(low), int(high or low)
        if first < 1 or last < first:
            raise ValueError(f"invalid map seed range: {part}")
        seeds += [seed for seed in range(first, last + 1) if seed not in seeds]
    if not seeds:
        raise ValueError("no map seeds selected")
    return tuple(seeds)


def identity(source, args):
    """Stable identity of one input: its source bytes and CLI arguments."""
    if isinstance(source, str):
        source = source.encode()
    record = {"source_sha256": hashlib.sha256(source).hexdigest(), "args": [str(arg) for arg in args]}
    return hashlib.sha256(json.dumps(record, sort_keys=True).encode()).hexdigest()


def _distinct(values):
    seen = []
    for value in values:
        if value not in seen:
            seen.append(value)
    return seen


class Oracle:
    """The seeded oracle a harness resamples with, checked once per run.

    `probe(binary)` captures one tiny deterministic input with the harness's
    own capture. When given, the seeded build with D2RS_GO_MAPSEED unset must
    reproduce --go's probe; otherwise every seeded output would differ from
    stock Go (for example a stock seeded build paired with a clock oracle)
    and the harness would report false Go nondeterminism.
    """

    def __init__(self, path, go=None, probe=None, build_command=BUILD_COMMAND):
        self.path = Path(path) if path else None
        self.go = go
        self.probe = probe
        self.build_command = build_command
        self._lock = threading.Lock()
        self._problem = None
        self._checked = False
        self._warned = False

    def problem(self):
        """None when usable, else why not (checked once, thread-safe)."""
        with self._lock:
            if not self._checked:
                self._checked = True
                if self.path is None:
                    self._problem = "no seedable-map Go oracle given"
                elif not self.path.is_file():
                    self._problem = f"seedable-map Go oracle not found: {self.path}"
                elif self.probe is not None and self.go is not None:
                    try:
                        paired = self.probe(self.path) == self.probe(self.go)
                    except Exception as error:  # noqa: BLE001 - reported as unusable
                        paired, detail = False, f" ({error})"
                    else:
                        detail = ""
                    if not paired:
                        self._problem = (f"seedable-map Go oracle {self.path} does not reproduce "
                                         f"--go {self.go} with {ENV} unset{detail}")
            return self._problem

    def message(self):
        return f"{self.problem()}; build it with: {self.build_command}"

    def warn_once(self):
        with self._lock:
            if self._warned:
                return
            self._warned = True
        print(f"error: a case needs the seedable-map Go oracle: {self.message()}", file=sys.stderr, flush=True)


@dataclass
class Resolution:
    classification: str
    seeds: list = field(default_factory=list)
    go_variants: int = 1
    seeds_tried: list = field(default_factory=list)
    seed_failures: list = field(default_factory=list)
    known: bool = False

    @property
    def passed(self):
        return self.classification in PASSING

    def record(self):
        return {"classification": self.classification, "mapseed_seeds": list(self.seeds),
                "go_variants": self.go_variants, "seeds_tried": list(self.seeds_tried),
                "seed_failures": list(self.seed_failures), "known_mapseed": self.known}


def resolve(stock, rust, run_seeded, oracle, seeds=DEFAULT_SEEDS, known=False, usable=None):
    """Classify one case from stock Go and d2-rs outcome keys.

    stock and rust are non-empty sequences of comparable outcome keys (one per
    repeat). run_seeded(seed) returns the key of the seeded oracle's run with
    D2RS_GO_MAPSEED=seed under the same capture. usable(key), when given,
    rejects seeded runs that timed out, crashed or wrote invalid output: they
    are recorded in seed_failures and are neither variants nor matches.
    Every seed runs (no early exit) so the record lists all matching seeds.
    """
    stock, rust = list(stock), list(rust)
    if not stock or not rust:
        raise ValueError("resolve needs at least one stock Go and one d2-rs outcome")
    rust_variants = _distinct(rust)
    stock_variants = _distinct(stock)
    if len(rust_variants) > 1:
        # d2-rs disagrees with itself: no Go seed can resolve that.
        return Resolution(MISMATCH, go_variants=len(stock_variants), known=known)
    stock_match = len(stock_variants) == 1 and stock_variants[0] == rust[0]
    if stock_match and not known:
        return Resolution(MATCH)
    if oracle is None or oracle.problem():
        if stock_match:
            return Resolution(MATCH, known=known)
        if oracle is not None:
            oracle.warn_once()
        return Resolution(ORACLE_MISSING, go_variants=len(stock_variants), known=known)
    seeds = list(seeds)
    seeded, failures = {}, []
    for seed in seeds:
        key = run_seeded(seed)
        if usable is not None and not usable(key):
            failures.append(seed)
        else:
            seeded[seed] = key
    variants = _distinct(stock + list(seeded.values()))
    matching = [seed for seed, key in seeded.items() if key == rust[0]]
    if matching:
        classification = KNOWN_MAPSEED if known else MAPSEED_MATCH
    elif stock_match:
        classification = MATCH
    elif len(variants) > 1:
        classification = NO_RUST_MATCH
    else:
        classification = MISMATCH
    return Resolution(classification, matching, len(variants), seeds, failures, known)


class Registry:
    """Known map-order-dependent inputs; read by every run, written only on request."""

    def __init__(self, path=REGISTRY):
        self.path = Path(path)
        self.entries = {}
        if self.path.is_file():
            data = json.loads(self.path.read_text())
            self.entries = {entry["identity"]: entry for entry in data.get("cases", [])}

    def known(self, case_identity):
        return case_identity in self.entries

    def add(self, case_identity, source, args, locator, reason, harness):
        if case_identity in self.entries:
            return False
        if isinstance(source, str):
            source = source.encode()
        self.entries[case_identity] = {
            "identity": case_identity, "source_sha256": hashlib.sha256(source).hexdigest(),
            "args": [str(arg) for arg in args], "harness": harness, "input": locator, "reason": reason,
            "recorded": datetime.date.today().isoformat()}
        return True

    def save(self):
        cases = sorted(self.entries.values(), key=lambda entry: (entry["harness"], entry["input"], entry["identity"]))
        document = {"description": "Inputs whose pinned Go output depends on map iteration order. "
                                   "Written only by a differential harness run with --record-mapseed; "
                                   "no oracle output is stored (see scripts/mapseed.py).",
                    "cases": cases}
        self.path.parent.mkdir(parents=True, exist_ok=True)
        temporary = self.path.with_name(self.path.name + ".tmp")
        temporary.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n")
        os.replace(temporary, self.path)


def reason_for(resolution):
    if resolution.classification == NO_RUST_MATCH:
        return "pinned Go output varies with map order; d2-rs matched no seeded output when recorded"
    return "pinned Go output varies with map order"


def record_cases(registry, cases, harness):
    """Adds every case whose Go output varied (two or more variants).

    cases: iterable of (identity, source, args, locator, Resolution).
    Returns the number of new entries; saves only when something was added.
    """
    added = 0
    for case_identity, source, args, locator, resolution in cases:
        if resolution is not None and resolution.go_variants > 1:
            added += registry.add(case_identity, source, args, locator, reason_for(resolution), harness)
    if added:
        registry.save()
    return added


def add_arguments(parser, oracle_help=None):
    parser.add_argument("--go-mapseed", type=Path, default=None,
                        help=oracle_help or "seedable-map Go oracle (default: the --go path plus -mapseed)")
    parser.add_argument("--mapseed-seeds", type=parse_seeds, default=DEFAULT_SEEDS,
                        help="map seeds to resample a differing case with (default 1-16)")
    parser.add_argument("--mapseed-registry", type=Path, default=REGISTRY,
                        help="registry of known map-order-dependent inputs")
    parser.add_argument("--record-mapseed", action="store_true",
                        help="add every case whose Go output varied to the registry")


def counts(records):
    """{classification: count} in a fixed order, from record() dicts or Resolutions."""
    result = {}
    for record in records:
        classification = record.classification if isinstance(record, Resolution) else record["classification"]
        result[classification] = result.get(classification, 0) + 1
    order = {name: index for index, name in enumerate(CLASSIFICATIONS)}
    return dict(sorted(result.items(), key=lambda item: (order.get(item[0], len(order)), item[0])))


def summary(label, total, records):
    """e.g. "dagre: 299/300 match, 1 mapseed-match (seeds 3,9)"."""
    tally = counts(records)
    parts = [f"{tally.pop(MATCH, 0)}/{total} match"]
    for classification, count in tally.items():
        seeds = sorted({seed for record in records
                        if (record.classification if isinstance(record, Resolution) else record["classification"]) == classification
                        for seed in (record.seeds if isinstance(record, Resolution) else record.get("mapseed_seeds", []))})
        note = f" (seeds {','.join(map(str, seeds))})" if seeds else ""
        parts.append(f"{count} {classification}{note}")
    return f"{label}: " + ", ".join(parts)
