# Engineering contract

This repository implements D2 in Rust. These rules apply to every change, by a
person or an agent.

## The oracle

- The compatibility target is upstream Go D2 `master` at the commit pinned in
  `reference-manifest.json`, including its bundled TALA layout engine. TALA is
  part of D2 parity, not a separate project.
- Move the pin only deliberately: update the manifest from the new upstream
  `go.mod`, rebase the patch series, regenerate the oracle fixtures and run
  every scoreboard. `scripts/regen-fixtures.py` refuses Go module versions
  that differ from the pin.
- Go and JS engines are test oracles only. Native algorithm paths are Rust; no
  Go code is linked into d2-rs.
- Parity is per architecture. Go fuses multiply-add on arm64 but not on amd64,
  and some `math` routines differ by architecture; d2-rs mirrors both with
  `cfg(target_arch)` paths. Compare a host's Rust output only against oracle
  captures built for the same architecture (`--goarch`, `docs/FIXTURES.md`).

## Choosing work

- Work is chosen by scoreboard, and every change names the failing boards it
  targets:
  - the upstream e2e goldens (`scripts/e2e-scoreboard.py`);
  - the oracle tier (`scripts/test-oracle.sh`);
  - the differential harnesses (`scripts/differential-scoreboard.py`,
    `scripts/wasm-differential-scoreboard.mjs`), where every distinct
    Go-vs-Rust mismatch cluster is a failing board;
  - performance (`scripts/perf-bench.py`, and `scripts/corpus-perf-sweep.py`
    over every e2e case and output format): every case must stay faster
    than Go;
  - the generated sweeps beyond the corpus (`scripts/scale-sweep.py`,
    `raster-stress.py`, `feature-stress.py`, `registry-sweep.py`,
    `chroma-differential.py`, `fuzz-differential.py`), whose open findings
    are listed in `docs/STATUS.md`.
- Every change keeps the other scoreboards unchanged: no new oracle-tier or
  default-tier failures, an unchanged e2e scoreboard, and no perf-bench case
  slower than Go or than before.
- Open-ended qualification of internal states without a failing board is not
  progress.

## Method

- Start from static comparison with the pinned Go source and fix the earliest
  divergence. No fixture-specific outputs, silent test exclusions or lossy
  normalization.
- Tests cover real contracts. Never edit a test's expected value or an oracle
  artifact to make a test pass; a value that restates old Go output is updated
  only to what the pinned Go produces, with the source cited.
- Diagnostic argument construction compiles out unless the
  `diagnostic-traces` feature is enabled.
- Preserve third-party and source notices (`THIRD_PARTY_NOTICES.md`).

## Fixtures and tests

- Oracle artifacts are regenerated, never committed (`docs/FIXTURES.md`). The
  repository stores the pin, the patches in `tools/patches/` listed in the
  manifest's `trace_patches`, recipes in `tools/fixtures/`, hooks,
  generators and small hand-written seeds. `scripts/regen-fixtures.py`
  rebuilds every Go/JS output into the ignored `oracle-cache/`.
- Each recipe's cache key names only what it depends on: upstream paths,
  module versions, toolchain, architecture, hook and patch hashes.
- Two test tiers. Plain `cargo test --workspace` is self-contained and needs no
  Go and no cache. Tests that read regenerated artifacts compile only under
  each crate's `oracle-fixtures` feature; with the feature on, a missing
  artifact fails loudly with its regen command.
- Recipe new fixtures only for behavior reachable from D2 input or public
  APIs and not already covered by upstream goldens.
- Raster parity captures require nonempty, complete output inventories, and
  validate every board before claiming a match (`collect_png_outputs` and
  `compare_png_outputs` in `scripts/check-render-parity.py`). Missing
  artifacts, corrupt output and timeouts are failures.

## Tooling

- Use uv for Python and pnpm for JavaScript tooling.
- Scripts take the pin, oracle binary and e2e case dump from
  `scripts/references.py`, never from hard-coded paths.
- External layout plugins (`d2plugin-*`) are out of scope; upstream master has
  removed them.
